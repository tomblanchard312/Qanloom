# Enki image client snapshot

`enki-client` and `enki-protocol` contain the client/protocol sources used by Qanloom's
existing Enki image integration. The source is copied without behavior changes (repository formatting applied) from the local
`tomblanchard312/Enki` checkout on 2026-10-30 (base commit `48abad7`, including local
image/protocol changes not yet published there). No daemon or inference implementation
is copied. The package manifests use explicit versions instead of inheriting Enki's
workspace, and the protocol path points to this snapshot.

Both shared SDK packages are MIT-licensed by explicit owner approval, with the grant recorded in their package LICENSE files. This does not change the daemon or inference implementation licenses.
Keep these files in sync with an explicitly reviewed Enki client revision; a clean clone
must not depend on an unpublished sibling checkout.

The owner has approved MIT licensing for these shared SDK packages. Their package manifests and included LICENSE files now record that grant; the daemon and native workers remain outside this snapshot and retain their existing terms.
