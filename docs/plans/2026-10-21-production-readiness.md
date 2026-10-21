# Production readiness (2026-10-21)

Up to four workstreams run at once. Items marked **Paul** need him (production access, money, or a decision).

## Wave 1 — now, in parallel
1. **Web app desktop support** (agent `web-desktop`, guildbyte-app branch `feat/desktop-hero-sync`): `desktop`
   client header, `CLIENT_RELEASES.desktop`, `GET /api/clients/desktop/update`, `GET /install.sh`,
   `GET /download/windows`, pairing device/tools and 410, docs.
2. **Release readiness** (agent `release-ready`, this repo): `release.sh --mac-only`, resource budget script, local
   end-to-end Mac run (install 0.1.0 → pair → count → update to 0.1.1) with a throwaway signing key.
3. **Public repo audit** (agent `repo-audit`, this repo): history and files checked for secrets and private data,
   README for outsiders. The licence choice is **Paul**'s.
4. ~~Plugin switch notice~~: dropped. The Claude plugin's hero has no users, so nobody needs a notice.

## Wave 2 — when a slot frees up
5. **Setup page** (web app): built, committed with wave 1's web work, checked live.
6. **Landing page download links** (ClashOfMeat/landing): mockup first.
7. **Web PR**: push `feat/desktop-hero-sync`, open the PR into `main`.

## Wave 3 — Paul
8. Review and merge the web PR; deploy; apply migration 067 on production.
9. Choose the licence and make `ClashOfMeat/client` public (install.sh and the .exe download straight from GitHub).
10. Give the production URL.
11. Decide on Linux and on paid code signing (Apple Developer ID, Windows certificate).

## Wave 4 — first release
12. `scripts/release.sh 0.1.0 --app <production URL>` (Mac and Windows), signed with Paul's key.
13. Production env: `CLASHOFMEAT_DESKTOP_STABLE_VERSION=0.1.0` and `CLASHOFMEAT_DESKTOP_STABLE_MANIFEST=<update.json URL>`.
14. Paul installs on his Mac and his Windows PC; archive ClashOfMeat/plugin once the setup page points to the app
    (it has no users, so nothing has to keep resolving).

## Status (2026-10-21, evening)

Ready to deploy; nothing in production exists yet.

- **Done:**
  - Web app: desktop work merged into `dev` (ClashOfMeat/app #13, #14, #15). CI is green and migrations 066–069 are
    applied locally. The launch steps are in ClashOfMeat/app `docs/desktop-launch.md`.
  - Landing: ClashOfMeat/landing #4 is merged into `main`. Its deploy failed for lack of Cloudflare secrets.
  - End-to-end on the Mac, all passing:
    - static update: install 0.1.0 → update to 0.1.1, bad signature rejected;
    - through the local web app: `/install.sh`, pairing, sync, update via `/api/clients/desktop/update`.
  - Release builds stay within the resource budget (`scripts/budget.sh`).
  - `release.sh 0.1.0 --app https://app.clashofmeat.com --dry-run` built both Mac archives and the Windows installer
    with the production URL. Its only blocker is the signing key (Paul's password).
- **Paul:**
  - DNS for clashofmeat.com and app.clashofmeat.com (NXDOMAIN today).
  - The VPS and its secrets on ClashOfMeat/app (`VPS_*`, `ENV_PROD`).
  - The Cloudflare secrets on ClashOfMeat/landing.
  - A Release run.
  - Then `scripts/release.sh 0.1.0 --app https://app.clashofmeat.com` with the key password.
  - Then the stable `ENV_PROD` values, the licence, and making this repo public.
- **Not tested:** Windows at runtime (after production is up, on Paul's PC), the Intel build at runtime, and the
  daemon restart after an update.
