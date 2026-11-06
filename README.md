# AlwaysDNS

A self-hosted filtering DNS resolver with a web dashboard, built for a homelab. The backend is Rust (axum, tokio, SQLite) and the frontend is React 19. It runs in Docker Compose from `deploy/`. See [PRD.md](PRD.md) for the full spec and [deploy/README.md](deploy/README.md) for setup.

## Contributing

Contributions are welcome: bug reports, ideas, docs fixes and code. Read [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up, run the checks and send a change.

## Why Rust

- **Memory safety in safe code.** The compiler rejects use-after-free and buffer overflows in safe Rust. Most high-severity bugs in large C/C++ codebases are memory-safety bugs (Microsoft and Google reports). This backend has no `unsafe` blocks, but bundled SQLite is C code.
- **No GC pauses.** Go's garbage collector caused latency spikes in Discord's service. This project has not measured its own tail latency.
- **Bounded memory.** The answer cache, query log and write queue have fixed size caps. Block lists are held in memory and grow with list size. One snapshot of the running container: 39 MiB of memory, 0.37% CPU. Long-term growth is not yet measured.
- **One binary.** The release build links statically on musl. The runtime image is Alpine, not scratch, at 65 MB on disk (18 MB compressed).

Rust does not make upstream-bound answers faster. Upstream round-trip time usually dominates a cache miss. Sources and caveats are in [PRD.md](PRD.md#why-rust).
