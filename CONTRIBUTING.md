# Contributing to AlwaysDNS

Thanks for wanting to help. AlwaysDNS is a self-hosted, filtering DNS resolver with a web dashboard, built for a homelab. Bug reports, ideas, documentation fixes and code are all welcome, and a small fix is as useful as a big feature.

## Ways to help

- **Report a bug.** Say what you did, what you expected and what happened. Include the version shown in the top bar and the relevant lines from `docker compose logs dnsui`.
- **Suggest an idea.** Describe the problem first, then the change you have in mind. See the "Still open" section of [PRD.md](PRD.md) for what is already planned, such as forced safe search.
- **Improve the docs.** If a step in [deploy/README.md](deploy/README.md) did not work for you, that is a bug worth reporting or fixing.
- **Test on your network.** Different routers, upstream resolvers and phones find problems one machine cannot.
- **Send code.** See below.

## Before you write code

For anything bigger than a small fix, open an issue first so we can agree on the approach before you spend time on it. A change that touches the DNS path (`backend/src/dns.rs`, `engine.rs`, `rules.rs`) deserves extra discussion, because every query on the network goes through it.

## Set up

The project has two parts. The frontend has its own README. The backend is configured through `backend/config.example.yaml`, which explains every setting.

| Part | Where | Run it |
|---|---|---|
| Backend (Rust: axum, tokio, SQLite, hickory-dns) | [backend/](backend/config.example.yaml) | `cargo run` with a config file |
| Frontend (React 19, Vite, TypeScript, TanStack Query and Table) | [frontend/](frontend/README.md) | `npm install`, then `npm run dev` |
| Full stack in Docker | [deploy/](deploy/README.md) | `docker compose up -d --build` |

Port 53 needs privileges on most systems. For local work, set `listen.dns` in your config to a high port such as `127.0.0.1:5399`. In dev, the frontend proxies `/api` to the backend. Point it somewhere else with `DNSUI_API_TARGET`.

## Check your change

Run these before you open a pull request. All of them should pass.

```sh
# backend
cd backend
cargo test
cargo clippy --all-targets

# frontend
cd frontend
npm run typecheck
npm test
npm run build
```

If you change something users can see, open it in a browser at desktop and phone width. A passing test run does not show a broken layout.

## Style and conventions

- **Commits.** Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `chore:`, `refactor:` and `test:`, with an optional scope such as `feat(frontend):`. Keep each commit to one logical change.
- **Keep changes small and on topic.** Do not reformat or refactor code you are not changing. Match the style around you.
- **Tests.** Add a test that fails without your change. A test should say why the behaviour matters, not only what it does.
- **Docs.** If behaviour changes, update the README or [PRD.md](PRD.md) in the same pull request.
- **Claims need evidence.** The docs make a few claims about Rust, memory and comparisons with other resolvers. Any number or comparison you add needs a source link or a measurement you can describe. "It is faster" is not enough.
- **Never commit secrets.** No passwords, API keys, `.env` files or real client IP addresses. Redact them in screenshots and logs. The dashboard's Mask IPs toggle helps.
- **No new dependencies without a reason.** Say in the pull request what the dependency does and why the standard library or an existing one is not enough.

## Security

Do not post a vulnerability, or a way to exploit one, in a public issue. Contact the maintainer through [their GitHub profile](https://github.com/tquizzle) instead.

## Pull requests

1. Fork the repository and make a branch.
2. Make your change and run the checks above.
3. Open a pull request. Describe what changed and why, and say how you tested it. Add a screenshot for visual changes.
4. Expect some back and forth. Review comments are about the change, not about you.

## Licence

The project does not have a licence file yet. If you plan a large contribution, ask the maintainer first.
