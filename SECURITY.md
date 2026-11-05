# Security Policy

## Scope

Footprints sits between AI agents and the disk. It stores copies of files, restores them on undo
and answers requests from agents through its MCP server. A flaw here can lose data, leak it or let
an agent write where it should not. Issues in scope include:

- An undo, a forced undo or a session undo that writes, deletes or overwrites a file other than
  the one recorded, for example through a crafted path, a symlink swap or a tampered ledger
- A blob hash or stored path that reaches outside the data directory when read
- A hook payload that makes `footprints hook` run a command, block the agent or corrupt the ledger
- The MCP server accepting a request that changes files beyond what `footprints_undo` documents
- Stored file contents readable by other users because of the permissions Footprints creates
- A conflict check that can be bypassed, so an undo silently overwrites a later change
- Any case where Footprints sends data off the machine

## Out of scope

- What an agent does before Footprints records it. The same goes for changes outside the watched
  directory. Both limits are documented in the README
- Secrets that an agent wrote into a file and that are then stored in the blob store as designed.
  Keeping the data directory private is covered in the README
- Network calls being impossible to undo
- Findings that need root access or physical access to the machine

## Supported versions

Only the latest release and the `main` branch get security fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately instead:

- Email: security@isaacadjei.me
- GitHub: [report a vulnerability](https://github.com/zaccesss/footprints/security/advisories/new)

Please include the version or commit, the operating system, the steps to reproduce and what an
attacker could achieve. Expect a reply within 7 days and a fix within 14 days of confirmation where
one is possible.

## The shared policy

> [!NOTE]
> The full policy, covering scope, the disclosure process and expected response times, is in
> [zaccesss/security-policy](https://github.com/zaccesss/security-policy) and on
> [isaacadjei.me/security-policy](https://isaacadjei.me/security-policy). This file takes
> precedence where the two differ.
