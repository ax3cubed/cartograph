# Security Policy

## Supported versions

Only the latest release receives security fixes.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Older releases | No |

## Reporting a vulnerability

Do not report a vulnerability in a public issue, pull request or discussion.

1. Go to the repository's [Security tab](https://github.com/ax3cubed/cartograph/security) and choose "Report a vulnerability". This opens a private report that only the maintainer can see.
2. If that option is unavailable, contact the maintainer privately through their GitHub profile: https://github.com/ax3cubed.

## What to include

- The version or commit you tested.
- What the problem is and what an attacker could do with it.
- Steps to reproduce it, or a proof of concept.
- Your environment, where it matters: operating system, browser, Node version, and how the app was run (`pnpm dev` or a production build).
- Any fix or mitigation you have in mind.

Cartograph is a small open-source project, so there is no guaranteed response time. Please allow time for a fix to be released before you publish details.

## Security-relevant configuration

From v0.2, Cartograph has one setting with a security consequence: `CARTOGRAPH_ALLOW_CLIENT_PROVIDER`.

That release lets the app send a question to a language model through a server route. The provider for that route is chosen as follows:

- If `CARTOGRAPH_AI_BASE_URL` is set on the server (with `CARTOGRAPH_AI_API_KEY` if the provider needs a key), the server always uses it. A provider supplied from the browser is ignored.
- If it is not set, a production server refuses any provider base URL supplied from the browser.
- Setting `CARTOGRAPH_ALLOW_CLIENT_PROVIDER=1` lifts that refusal. The server will then fetch whatever base URL the browser supplies.

That last option is intended for single-user local installs only. A server that fetches a user-supplied URL can be pointed at addresses on its own network, which is a server-side request forgery risk. The switch is an opt-in and does not filter addresses. Leave it unset on any deployment that other people can reach, and configure the provider with `CARTOGRAPH_AI_BASE_URL` there.

`next dev` accepts a provider from the browser without the switch. Do not expose a development server to a network you do not trust.

v0.1.0 has no such setting. It makes no model calls and has no server route that fetches external URLs.
