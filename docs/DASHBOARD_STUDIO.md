# Infra Studio — identity and dashboard redesign

**Status:** New design on an isolated branch. This is a development preview, not yet deployed to the live gateway.

## Creative direction

Inspired by the design *principles* found in these Dribbble works, without using or copying their design assets:

- [Minimal SaaS Dashboard — Bea Grey](https://dribbble.com/shots/26989557-Minimal-SaaS-Dashboard-Clean-UI-Visual-Hierarchy): space, readable type, deliberate contrast.
- [Sleek & Intuitive SaaS Dashboard — Azaz](https://dribbble.com/shots/25793491-Sleek-Intuitive-SaaS-Dashboard-UI): disciplined navigation and consistently treated interactions.
- [FutureTech Monogram — Nixzora](https://dribbble.com/shots/27181569-FutureTech-Monogram-Logo-Minimal-SaaS-Brand-Identity): scalable geometric identity.
- [Nyerver Server Monitoring — One Week Wonders](https://dribbble.com/shots/9873683-Nyerver-Server-Monitoring-Dashboard): purposeful device-management surfaces, adapted without gratuitous charts.

The visual system has been rebuilt in graphite and muted periwinkle, with a balanced two-column device/work canvas, spacious four-figure operational summary, responsive compact navigation, and an original geometric bridge logomark.

## New Infra logo

`apps/dashboard/assets/logo.svg` is the vector source. Two architectural rails meet at a pale-blue bridge, representing devices joined through a privately controlled gateway. Matching raster files support the app wordmark, favicon and mobile touch icon. The images and icons are local, not external assets.

## Real browser renders (fixture data)

### Desktop overview

![Desktop](previews/studio/overview-desktop.png)

### Mobile overview

![Mobile](previews/studio/overview-mobile.png)

### Sign-in

![Sign-in](previews/studio/sign-in.png)

### Mobile device details

![Device details](previews/studio/device-mobile.png)

## Safety and validation

The backend contract, authenticated owner pairing, terminal attach and remote operations, file browser, persistent sessions, and logout are unchanged. The browser suite verifies these interactions, hostile-name escaping, locally loaded brand image, CSP, and no layout overflow at 320/390/768/1280 px. The Rust gateway tests and strict lint pass. No new network font, JavaScript, dependency or third-party image was introduced.

The source branch was isolated from unrelated, uncommitted installation changes in the original worktree. The live gateway should be updated only after a fresh database snapshot, verified staging against the live schema, inactive remote work, signed agent reconnection acceptance, and rollback preparation.
