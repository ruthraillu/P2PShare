# P2PShare signaling Worker

P2PShare uses a Cloudflare Worker with Durable Objects for peer discovery and WebRTC SDP/ICE signaling.

Windows and Android clients connect to `/v3/rooms/` to register users automatically by room name. There are no passwords or Create operations, and named rooms have no fixed participant cap. Signed device identities, session replacement, WebSocket hibernation, and directed SDP remain. Screen video stays direct encrypted WebRTC; this Worker provides no video relay or TURN. `v1` invitation connections remain for compatibility and transport regression tests, while the retired `/v2/rooms/` password endpoint returns `410`.

## Self-hosting setup

From the `server/signaling-worker` directory in PowerShell (requires Node.js):

```powershell
.\worker.ps1 -Action Install
.\worker.ps1 -Action Test
.\worker.ps1 -Action Check
.\worker.ps1 -Action Login
.\worker.ps1 -Action Deploy
```

- `Install`: installs dependencies from `package-lock.json` (`npm ci`).
- `Test`: runs the local Miniflare test suite (`tests/*.test.mjs`).
- `Check`: builds a Wrangler dry-run deployment bundle locally.
- `Login`: authenticates Wrangler with your Cloudflare account.
- `Deploy`: deploys the `p2pshare-signaling` Worker and its `Room` / `DiagnosticsQuota` Durable Objects to Cloudflare.

Once deployed, enter your Worker's `wss://<your-worker>.workers.dev` URL in P2PShare when prompted for the signaling server (or set `signaling_url` in `config.toml`).

## Optional private diagnostics

In Cloudflare, create a Workers KV namespace (for example `p2pshare-diagnostics`) in the same account as your signaling Worker. Copy its namespace ID and add this top-level entry to your local `wrangler.jsonc`:

```jsonc
"kv_namespaces": [{ "binding": "DIAGNOSTICS", "id": "YOUR_NAMESPACE_ID" }]
```

Then deploy the updated configuration:

```powershell
.\worker.ps1 -Action Deploy
```

In the Worker dashboard under **Settings → Variables and Secrets**, add a text variable named `DIAGNOSTICS_ENABLED` with the value `true` to open uploads. Set it to `false` or remove it to close uploads. The default is closed.

Desktop clients reuse their configured secure signaling URL and post to `/diagnostics`. While open, the route accepts uploads without credentials. Each report expires automatically after 7 days. Uploads are capped at 4 MiB per report and a shared 32 reports or 64 MiB per UTC day. A failed storage write still consumes its quota reservation.

Select a run in **Settings → Diagnostics** and click **Send diagnostics**. Reports include exact IP addresses and the current Windows username. Find the full KV key by matching the upload timestamp and short report ID shown in the app (the key also includes the sending Windows device name). Download by full key with Wrangler while signed in to Cloudflare (run from Command Prompt in `server/signaling-worker` so binary output is preserved):

```cmd
npx wrangler kv key get "<full-kv-key>" --binding DIAGNOSTICS --remote > "<full-kv-key>"
```
