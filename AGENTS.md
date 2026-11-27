# Repository agent rules
- On-device only: no remote execution or cloud-hosted terminal, IDE, agent harness, MCP stdio or Docker. Model inference APIs and normal network downloads may be allowed with explicit permissions.
- Prefer official harnesses and documented protocols; no reverse-engineered auth or proprietary binary redistribution without permission.
- Do not claim capabilities before device evidence. Unimplemented features fail closed; mocks are for tests only.
- Keep user project data separate from secrets and IDE internals. Do not expose native workspace API as a security boundary against hostile guest processes.
- Preserve upstream licenses; pin tested versions and record provenance. Do not execute unknown install scripts automatically.
- Run swift test for core changes; record iOS simulator and physical-device validation separately.
