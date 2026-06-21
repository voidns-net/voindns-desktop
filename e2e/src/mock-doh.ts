// Minimal RFC 8484 DNS-over-HTTPS server used as the e2e upstream.
//
// It answers every A query with 192.0.2.1 (TEST-NET-1) and records every
// question name it receives. The whole point of the test is step 6: assert a
// query we fired at the proxy actually arrived here over DoH.
//
// Must speak HTTP/2: hickory-dns's DoH client negotiates ALPN `h2`, so a plain
// HTTP/1.1 `https` server would fail the handshake and the proxy would ServFail.
// `allowHTTP1: true` keeps a GET/POST fallback path working too.

import http2 from "node:http2";
import { Buffer } from "node:buffer";
import dnsPacket from "dns-packet";

export interface MockDoh {
  readonly port: number;
  /** Lower-cased question names seen so far, in arrival order. */
  readonly seen: string[];
  /** Resolve true once `name` has been seen, or false on timeout. */
  waitFor(name: string, timeoutMs: number): Promise<boolean>;
  close(): Promise<void>;
}

const DOH_CT = "application/dns-message";

function buildResponse(query: dnsPacket.DecodedPacket): Buffer {
  const questions = query.questions ?? [];
  const answers = questions
    .filter((q) => q.type === "A")
    .map((q) => ({
      type: "A" as const,
      class: "IN" as const,
      name: q.name,
      ttl: 60,
      data: "192.0.2.1",
    }));
  return dnsPacket.encode({
    type: "response",
    id: query.id ?? 0,
    flags: dnsPacket.RECURSION_DESIRED | dnsPacket.RECURSION_AVAILABLE,
    questions,
    answers,
  });
}

/** A uniform "respond with these headers + body" sink for both h2 and h1. */
type Responder = (status: number, headers: Record<string, string>, body?: Buffer) => void;

export async function startMockDoh(opts: {
  cert: string;
  key: string;
  port: number;
}): Promise<MockDoh> {
  const seen: string[] = [];

  const handle = (rawPath: string, method: string, body: Buffer, respond: Responder) => {
    const url = new URL(rawPath || "/", "https://localhost");
    if (!url.pathname.endsWith("/dns-query")) return respond(404, {});

    let wire: Buffer | null = null;
    if (method === "GET") {
      const b64 = url.searchParams.get("dns"); // RFC 8484 §4.1 base64url
      if (b64) wire = Buffer.from(b64, "base64url");
    } else if (method === "POST") {
      wire = body;
    } else {
      return respond(405, {});
    }
    if (!wire) return respond(400, {});

    let reply: Buffer;
    try {
      const msg = dnsPacket.decode(wire);
      for (const q of msg.questions ?? []) seen.push(q.name.toLowerCase());
      reply = buildResponse(msg);
    } catch {
      return respond(400, {});
    }
    respond(200, { "content-type": DOH_CT, "content-length": String(reply.length) }, reply);
  };

  const server = http2.createSecureServer({
    cert: opts.cert,
    key: opts.key,
    allowHTTP1: true,
  });

  // HTTP/2 path (what the proxy uses).
  server.on("stream", (stream, headers) => {
    const method = String(headers[":method"] ?? "GET");
    const rawPath = String(headers[":path"] ?? "/");
    const chunks: Buffer[] = [];
    stream.on("data", (c: Buffer) => chunks.push(c));
    stream.on("end", () => {
      handle(rawPath, method, Buffer.concat(chunks), (status, hdrs, body) => {
        stream.respond({ ":status": status, ...hdrs });
        stream.end(body);
      });
    });
    stream.on("error", () => {});
  });

  // HTTP/1.1 fallback (allowHTTP1).
  server.on("request", (req, res) => {
    if (req.httpVersionMajor >= 2) return; // handled by the 'stream' listener
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      handle(req.url ?? "/", req.method ?? "GET", Buffer.concat(chunks), (status, hdrs, body) => {
        res.writeHead(status, hdrs);
        if (body) res.end(body);
        else res.end();
      });
    });
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(opts.port, "127.0.0.1", resolve);
  });

  return {
    port: opts.port,
    seen,
    async waitFor(name, timeoutMs) {
      const target = name.toLowerCase().replace(/\.$/, "");
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (seen.some((s) => s.replace(/\.$/, "") === target)) return true;
        await new Promise((r) => setTimeout(r, 100));
      }
      return false;
    },
    close() {
      return new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
