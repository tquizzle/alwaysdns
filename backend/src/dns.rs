//! The DNS request handler. Each query goes through: policy (engine), then
//! rewrite or block, then cache, then DoH forwarding, then telemetry.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use hickory_proto::op::{Edns, Header, HeaderCounts, Metadata, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
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
    let mut servers: Vec<String> = rt
        .config
        .upstream_groups
        .get(group)
        .cloned()
        .unwrap_or_default();
    if let Some(defaults) = rt.config.upstream_groups.get(&rt.config.default_upstream_group) {
        for url in defaults {
            if !servers.contains(url) {
                servers.push(url.clone());
            }
        }
    }
    servers
}
