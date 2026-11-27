// Diretório de ID curto do Elo.
//
// Cada aparelho tem um identificador estável e secreto (`device`): o SHA-256,
// em hex, de um ID do hardware/sistema com sal do Elo (Windows: MachineGuid;
// Android: ANDROID_ID). Ele sobrevive a reinstalar o app. O servidor associa
// esse identificador a um número curto de 9 dígitos que NUNCA muda, e guarda
// junto a chave extensa (endereço P2P iroh) atual, que muda a cada reinstalação.
//
//   POST /v1/register  {"device":"<64 hex>","node":"<endereço P2P>"}
//        -> 200 {"short":"123456789"}      (mesmo número para o mesmo device)
//   GET  /v1/resolve/<9 dígitos>
//        -> 200 {"node":"<endereço P2P>"} | 404
//
// KV (gravações só quando algo muda, para caber no plano gratuito):
//   d:<device> -> {"short","node","updated"}
//   s:<short>  -> {"node","updated"}        (sem o device: resolve não o expõe)

const DEVICE_RE = /^[0-9a-f]{64}$/;
const NODE_RE = /^[A-Za-z0-9_-]{16,128}$/;
const SHORT_RE = /^[1-9][0-9]{8}$/;

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function randomShort() {
  const b = new Uint32Array(1);
  crypto.getRandomValues(b);
  return String(100000000 + (b[0] % 900000000));
}

async function register(req, env) {
  let body;
  try {
    body = await req.json();
  } catch {
    return json({ error: "JSON inválido" }, 400);
  }
  const device = String(body.device || "").toLowerCase();
  const node = String(body.node || "");
  if (!DEVICE_RE.test(device)) return json({ error: "device inválido" }, 400);
  if (!NODE_RE.test(node)) return json({ error: "node inválido" }, 400);

  const now = Date.now();
  const existing = await env.DIR.get("d:" + device, "json");
  if (existing && SHORT_RE.test(existing.short)) {
    if (existing.node !== node) {
      // Reinstalação: mesma máquina, chave nova. Mantém o número curto.
      await env.DIR.put("s:" + existing.short, JSON.stringify({ node, updated: now }));
      await env.DIR.put("d:" + device, JSON.stringify({ short: existing.short, node, updated: now }));
    }
    return json({ short: existing.short });
  }

  for (let i = 0; i < 10; i++) {
    const short = randomShort();
    if (await env.DIR.get("s:" + short)) continue;
    await env.DIR.put("s:" + short, JSON.stringify({ node, updated: now }));
    await env.DIR.put("d:" + device, JSON.stringify({ short, node, updated: now }));
    return json({ short });
  }
  return json({ error: "não foi possível alocar um número" }, 503);
}

async function resolve(short, env) {
  if (!SHORT_RE.test(short)) return json({ error: "número inválido" }, 400);
  const rec = await env.DIR.get("s:" + short, "json");
  if (!rec) return json({ error: "número não encontrado" }, 404);
  return json({ node: rec.node });
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (req.method === "POST" && url.pathname === "/v1/register") return register(req, env);
    const m = url.pathname.match(/^\/v1\/resolve\/([0-9]+)$/);
    if (req.method === "GET" && m) return resolve(m[1], env);
    if (req.method === "GET" && url.pathname === "/") return json({ service: "elo-directory", version: 1 });
    return json({ error: "não encontrado" }, 404);
  },
};
