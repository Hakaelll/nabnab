# Gallery authentication and direct attachments

Register `gallery_auth` and its Tauri commands `set_gallery_token` and
`gallery_auth_status`. The Accounts panel can mount `createGalleryAuth` from
`src/gallery-auth-ui.ts`; call `load()` when opening Accounts, render its panel,
and route its submit/click handlers through the existing error boundary.

Pixiv artworks require the user's refresh token; browser cookies alone do not
authorize the pinned gallery-dl Pixiv API extractor. The token is stored with the
existing protected module-secret mechanism. A private, exclusive temporary JSON
configuration is passed by filename with `-c`, never by token in arguments. Its
RAII guard removes it; interrupted files share the session cleanup prefix. The
configuration is explicitly loaded after `--ignore-config`. Errors are redacted
against the active token. Public Sketch uses its independent extractor.
The engine's persistent login/token cache is disabled with `cache.file=null`;
ignoring user configuration alone would still leave gallery-dl's default cache active.

Both gallery inspection and followed-gallery checks request Instagram's `merged`
video mode. The pinned default emits synthetic `ytdl:` URLs plus embedded DASH
metadata; stripping that prefix would create an invalid direct download. Other
manifest-only entries are excluded with an explicit warning in manual inspection.
Following fails with an actionable limitation when such entries prevent complete
preservation. This route does not promise HLS/DASH merging, Pixiv ugoira ZIP
conversion, or arbitrary extractor postprocessors.

`Asset.referer` and `Asset.source_url` are backend-only. The sole approved Referer
is the fixed public `https://www.pixiv.net/`, when an actual Pixiv source yields
an HTTPS pximg.net attachment. No arbitrary extractor headers or cross-origin
cookies are carried to the direct downloader. `source_url` removes query and
fragment and is provenance, not an operational download link. Parent queue
integration must persist these fields and use an origin-checked direct-download
Referer; a bare CDN URL loses the extractor context otherwise.

Gallery `warnings` must be displayed next to inspection results. Cursor positions
and `hasMore` use raw extractor file positions, including excluded formats.
Completed jobs only suppress another download while their output files exist.

Official pinned sources: [Instagram](https://raw.githubusercontent.com/mikf/gallery-dl/v1.31.10/gallery_dl/extractor/instagram.py),
[Pixiv](https://raw.githubusercontent.com/mikf/gallery-dl/v1.31.10/gallery_dl/extractor/pixiv.py),
[Twitter](https://raw.githubusercontent.com/mikf/gallery-dl/v1.31.10/gallery_dl/extractor/twitter.py).
Twitter emits `ytdl:` when its video or card options explicitly request that
downloader; its default native video variant has an ordinary media URL.
