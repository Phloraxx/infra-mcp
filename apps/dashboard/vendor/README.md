Terminal rendering uses the unmodified browser bundle and stylesheet from
`@xterm/xterm` 5.5.0, distributed under the MIT license in `LICENSE.xterm`.

Source: https://github.com/xtermjs/xterm.js/tree/5.5.0
Package: https://registry.npmjs.org/@xterm/xterm/-/xterm-5.5.0.tgz
Package SHA-1 (npm): `275fb8f6e14afa6e8a0c05d4ebc94523ff775396`

Only `lib/xterm.js`, `css/xterm.css`, and the license are vendored. The gateway
serves them locally; the dashboard makes no CDN requests. The source-map comment
is retained in the original bundle, but the development source map is not shipped.
