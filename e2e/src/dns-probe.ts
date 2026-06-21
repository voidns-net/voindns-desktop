// Fire a plain DNS query at the proxy's loopback listener and (best-effort) read
// the reply. The real assertion is that the mock DoH server saw the name; the
// UDP reply just confirms the proxy answered (192.0.2.1 from the mock).

import dgram from "node:dgram";
import { Buffer } from "node:buffer";
import dnsPacket from "dns-packet";

export interface ProbeResult {
  /** The randomized FQDN we asked for. */
  name: string;
  /** A-records in the reply, if one arrived in time. */
  answers: string[];
}

/** A unique name per run so the proxy cache never masks a fresh upstream hit. */
export function randomProbeName(): string {
  const rand = Math.random().toString(36).slice(2, 10);
  return `probe-${rand}.e2e.voidns.test`;
}

export async function probe(opts: {
  host: string;
  port: number;
  name: string;
  timeoutMs: number;
}): Promise<ProbeResult> {
  const socket = dgram.createSocket("udp4");
  const query = dnsPacket.encode({
    type: "query",
    id: (Math.random() * 0xffff) | 0,
    flags: dnsPacket.RECURSION_DESIRED,
    questions: [{ type: "A", name: opts.name }],
  });

  const answers: string[] = [];
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => resolve(), opts.timeoutMs); // soft timeout
      socket.once("error", (e) => {
        clearTimeout(timer);
        reject(e);
      });
      socket.on("message", (msg: Buffer) => {
        try {
          const reply = dnsPacket.decode(msg);
          for (const a of reply.answers ?? []) {
            if (a.type === "A") answers.push(String(a.data));
          }
        } catch {
          /* ignore malformed */
        }
        clearTimeout(timer);
        resolve();
      });
      socket.send(query, opts.port, opts.host, (e) => {
        if (e) {
          clearTimeout(timer);
          reject(e);
        }
      });
    });
  } finally {
    socket.close();
  }

  return { name: opts.name, answers };
}
