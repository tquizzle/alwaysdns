//! The DNS request handler. Each query goes through: policy (engine), then
//! rewrite or block, then cache, then DoH forwarding, then telemetry.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use hickory_proto::op::{Edns, Header, HeaderCounts, Metadata, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA, CNAME};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_server::net::runtime::Time;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::zone_handler::MessageResponseBuilder;
use tracing::{debug, warn};

use crate::app::AppState;
use crate::cache::make_key;
use crate::engine::{Decision, Runtime, Verdict};
use crate::recorder::QueryRow;
use crate::rules::{canonical_ip, normalize_name};
use crate::telemetry::QueryEvent;

/// TTL of local rewrite answers, in seconds.
const REWRITE_TTL: u32 = 60;

/// The first address in an answer, as text. Used for the destination map.
fn first_answer_ip(answers: &[Record]) -> Option<String> {
    answers.iter().find_map(|r| match &r.data {
        RData::A(a) => Some(a.0.to_string()),
        RData::AAAA(a) => Some(a.0.to_string()),
        _ => None,
    })
}

pub struct DnsHandler {
    state: Arc<AppState>,
}

impl DnsHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    async fn resolve(&self, rt: &Runtime, decision: &Decision, name: &str, qtype: RecordType) -> Outcome {
        let allowed = decision.verdict == Verdict::Allowed;
        match &decision.verdict {
            Verdict::Rewrite(addr) => Outcome {
                answers: rewrite_answers(name, *addr, qtype),
                rcode: ResponseCode::NoError,
                result: "rewritten",
                upstream: "local".to_string(),
            },
            Verdict::SafeSearch(found) => {
                self.safe_search_outcome(rt, &decision.group, name, qtype, found.target)
                    .await
            }
            Verdict::Blocked => Outcome {
                answers: Vec::new(),
                rcode: ResponseCode::NXDomain,
                result: "blocked",
                upstream: "-".to_string(),
            },
            Verdict::Allowed | Verdict::Pass => {
                let key = make_key(&decision.group, name, qtype);
                if let Some(hit) = self.state.cache.get(&key) {
                    return Outcome {
                        answers: hit.answers,
                        rcode: hit.rcode,
                        result: if allowed { "allowed" } else { "cached" },
                        upstream: "cache".to_string(),
                    };
                }

                let servers = servers_for_group(rt, &decision.group);
                match self.state.upstreams.forward(&servers, name, qtype).await {
                    Ok(ok) => {
                        self.state.cache.put(key, &ok.answers, ok.rcode);
                        Outcome {
                            answers: ok.answers,
                            rcode: ok.rcode,
                            result: if allowed { "allowed" } else { "forwarded" },
                            upstream: ok.upstream,
                        }
                    }
                    Err(err) => {
                        warn!(%name, %qtype, error = %err, "all upstreams failed");
                        Outcome {
                            answers: Vec::new(),
                            rcode: ResponseCode::ServFail,
                            result: "error",
                            upstream: "-".to_string(),
                        }
                    }
                }
            }
        }
    }

    /// Answers a safe-search name as an alias of `target`, looked up through the
    /// client's upstream group and the shared cache.
    async fn safe_search_outcome(
        &self,
        rt: &Runtime,
        group: &str,
        name: &str,
        qtype: RecordType,
        target: &str,
    ) -> Outcome {
        // Other types (HTTPS, SVCB and so on) get an empty answer. Forwarding them
        // would hand back the provider's real, unfiltered records.
        if !matches!(qtype, RecordType::A | RecordType::AAAA) {
            return Outcome {
                answers: Vec::new(),
                rcode: ResponseCode::NoError,
                result: "rewritten",
                upstream: "local".to_string(),
            };
        }
        let key = make_key(group, target, qtype);
        let (found, rcode, upstream) = if let Some(hit) = self.state.cache.get(&key) {
            (hit.answers, hit.rcode, "cache".to_string())
        } else {
            let servers = servers_for_group(rt, group);
            match self.state.upstreams.forward(&servers, target, qtype).await {
                Ok(ok) => {
                    self.state.cache.put(key, &ok.answers, ok.rcode);
                    (ok.answers, ok.rcode, ok.upstream)
                }
                Err(err) => {
                    warn!(%name, %target, error = %err, "safe search target lookup failed");
                    return Outcome {
                        answers: Vec::new(),
                        rcode: ResponseCode::ServFail,
                        result: "error",
                        upstream: "-".to_string(),
                    };
                }
            }
        };
        Outcome {
            answers: safe_search_answers(name, target, found, rcode),
            rcode,
            result: "rewritten",
            upstream,
        }
    }

    async fn send<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: &mut R,
        answers: Vec<Record>,
        rcode: ResponseCode,
    ) -> ResponseInfo {
        let mut metadata = Metadata::response_from_request(&request.metadata);
        metadata.response_code = rcode;
        metadata.recursion_available = true;
        metadata.authoritative = false;

        let mut edns = Edns::new();
        edns.set_max_payload(1232);
        edns.set_version(0);
        let edns_ref = if request.edns.is_some() { Some(&edns) } else { None };

        let response = MessageResponseBuilder::new(&request.queries, edns_ref).build(
            metadata,
            answers.iter(),
            std::iter::empty::<&Record>(),
            std::iter::empty::<&Record>(),
            std::iter::empty::<&Record>(),
        );
        match response_handle.send_response(response).await {
            Ok(info) => info,
            Err(err) => {
                warn!(error = %err, "failed to send DNS response");
                serve_failed(request)
            }
        }
    }
}

struct Outcome {
    answers: Vec<Record>,
    rcode: ResponseCode,
    result: &'static str,
    upstream: String,
}

#[async_trait::async_trait]
impl RequestHandler for DnsHandler {
    async fn handle_request<R: ResponseHandler, T: Time>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let started = Instant::now();

        if request.metadata.op_code != OpCode::Query {
            return self
                .send(request, &mut response_handle, Vec::new(), ResponseCode::NotImp)
                .await;
        }
        let Some(query) = request.queries.queries().first() else {
            return self
                .send(request, &mut response_handle, Vec::new(), ResponseCode::FormErr)
                .await;
        };

        let name = normalize_name(&query.name().to_string());
        let qtype = query.query_type();
        let src: IpAddr = canonical_ip(request.src().ip());

        let rt = self.state.runtime();
        let decision = rt.decide(src, &name);
        let outcome = self.resolve(&rt, &decision, &name, qtype).await;

        debug!(
            client = ?decision.client,
            %name,
            %qtype,
            result = outcome.result,
            upstream = %outcome.upstream,
            "query"
        );
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.state.telemetry.record(QueryEvent {
            client_ip: src,
            client: decision.client.clone(),
            domain: name.clone(),
            qtype: qtype.to_string(),
            result: outcome.result,
            upstream: outcome.upstream.clone(),
            elapsed_ms,
            rule: decision.rule.clone(),
        });
        // Non-blocking: a full channel drops the row and never delays the answer.
        if let Some(sink) = &self.state.log_sink {
            sink.record(QueryRow {
                time_ms: chrono::Utc::now().timestamp_millis(),
                client_ip: src.to_string(),
                client_name: decision.client.clone().unwrap_or_else(|| "unknown".to_string()),
                domain: name.clone(),
                qtype: qtype.to_string(),
                result: outcome.result.to_string(),
                upstream: outcome.upstream.clone(),
                elapsed_ms,
                rule: decision.rule.clone(),
                answer_ip: first_answer_ip(&outcome.answers),
            });
        }

        self.send(request, &mut response_handle, outcome.answers, outcome.rcode)
            .await
    }
}

/// Response info used when the reply could not be written to the socket.
fn serve_failed(request: &Request) -> ResponseInfo {
    let mut metadata = Metadata::response_from_request(&request.metadata);
    metadata.response_code = ResponseCode::ServFail;
    ResponseInfo::from(Header {
        metadata,
        counts: HeaderCounts::default(),
    })
}

/// Builds the local answer for a rewrite. Only A (for IPv4) and AAAA (for
/// IPv6) produce records. Any other type, or a family mismatch, gets NODATA.
fn rewrite_answers(name: &str, addr: IpAddr, qtype: RecordType) -> Vec<Record> {
    let Ok(owner) = Name::from_ascii(format!("{name}.")) else {
        return Vec::new();
    };
    match (addr, qtype) {
        (IpAddr::V4(v4), RecordType::A) => {
            vec![Record::from_rdata(owner, REWRITE_TTL, RData::A(A(v4)))]
        }
        (IpAddr::V6(v6), RecordType::AAAA) => {
            vec![Record::from_rdata(owner, REWRITE_TTL, RData::AAAA(AAAA(v6)))]
        }
        _ => Vec::new(),
    }
}

/// The group's servers, followed by the default group's servers as a fallback
/// (duplicates removed). Used when every server in the group fails.
fn servers_for_group(rt: &Runtime, group: &str) -> Vec<String> {
    let mut servers = rt.config.group_urls(group);
    for url in rt.config.group_urls(&rt.config.default_upstream_group) {
        if !servers.contains(&url) {
            servers.push(url);
        }
    }
    servers
}

/// The answer for a safe-search name: a CNAME to `target`, then the target's own
/// records. If the target lookup failed, there are no answers and the caller
/// passes the same response code on, so the client sees the real failure.
fn safe_search_answers(
    name: &str,
    target: &str,
    found: Vec<Record>,
    rcode: ResponseCode,
) -> Vec<Record> {
    if rcode != ResponseCode::NoError {
        return Vec::new();
    }
    let (Ok(owner), Ok(host)) = (
        Name::from_ascii(format!("{name}.")),
        Name::from_ascii(format!("{target}.")),
    ) else {
        return Vec::new();
    };
    let ttl = found.iter().map(|r| r.ttl).min().unwrap_or(REWRITE_TTL);
    let mut answers = Vec::with_capacity(found.len() + 1);
    answers.push(Record::from_rdata(owner, ttl, RData::CNAME(CNAME(host))));
    answers.extend(found);
    answers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const TARGET: &str = "forcesafesearch.google.com";

    fn a_record(ttl: u32, last: u8) -> Record {
        let owner = Name::from_ascii(format!("{TARGET}.")).unwrap();
        Record::from_rdata(owner, ttl, RData::A(A(Ipv4Addr::new(203, 0, 113, last))))
    }

    #[test]
    fn safe_search_answer_is_a_cname_followed_by_the_target_records() {
        let out = safe_search_answers("www.google.com", TARGET, vec![a_record(120, 9)], ResponseCode::NoError);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name.to_ascii(), "www.google.com.");
        assert!(matches!(&out[0].data, RData::CNAME(c) if c.0.to_ascii() == format!("{TARGET}.")));
        assert!(matches!(out[1].data, RData::A(_)));
    }

    #[test]
    fn the_alias_ttl_is_the_shortest_target_ttl() {
        let found = vec![a_record(300, 1), a_record(45, 2)];
        let out = safe_search_answers("www.google.com", TARGET, found, ResponseCode::NoError);
        assert_eq!(out[0].ttl, 45);
    }

    #[test]
    fn a_target_with_no_records_still_answers_with_the_alias() {
        // An AAAA query for a host with no IPv6 address: alias only, NOERROR, no leak.
        let out = safe_search_answers("www.google.com", TARGET, Vec::new(), ResponseCode::NoError);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ttl, REWRITE_TTL);
    }

    #[test]
    fn a_failed_target_lookup_returns_no_answers() {
        let out = safe_search_answers("www.google.com", TARGET, vec![a_record(60, 3)], ResponseCode::NXDomain);
        assert!(out.is_empty());
    }
}
