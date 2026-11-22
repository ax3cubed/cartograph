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

The app is a static export (`out/`). It has no server routes, makes no model calls and fetches nothing outside its own origin. The Cartograph binary serves it on `127.0.0.1` only.

`pnpm build` writes `out/csp.json`: a Content-Security-Policy template and, for each page, the SHA-256 hashes of its inline scripts. The server fills the template per page, so scripts run without `'unsafe-inline'` and `connect-src 'self'` keeps the browser from calling any other host.

v0.2.0 had a server route, `POST /api/ai`, and a setting `CARTOGRAPH_ALLOW_CLIENT_PROVIDER` that let it fetch a base URL supplied by the browser (a server-side request forgery risk). Both are removed. Model calls return in the binary, which will hold the provider configuration itself.

Do not expose `pnpm dev` to a network you do not trust.
