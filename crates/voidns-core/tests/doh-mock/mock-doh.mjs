// Local RFC 8484 DoH resolver for the Rust hermetic tests. Vendored (and reused)
// from the backend browser-doh suite: backend/tests/browser-doh/src/mock-doh.ts.
//
// Answers every query with a single A record (127.0.0.1) over HTTP/2 TLS, using
// the cert/key passed on the CLI. h2 is advertised first (DoH clients negotiate
// HTTP/2); http/1.1 is also allowed. Prints `PORT=<n>` once bound so the Rust
// test can read the ephemeral port, then serves until killed.
//
//   node mock-doh.mjs <certPath> <keyPath> [port]
//
import { createSecureServer } from 'node:http2'
import { readFileSync } from 'node:fs'
import { Buffer } from 'node:buffer'
import dnsPacket from 'dns-packet'

function namesOf(buf) {
  try {
    const query = dnsPacket.decode(buf)
    const questions = query.questions ?? []
    return { id: query.id ?? 0, questions, names: questions.map((q) => (q.name ?? '').toLowerCase()).filter(Boolean) }
  } catch {
    return null
  }
}

function start(certPath, keyPath, port = 0) {
  const server = createSecureServer({
    key: readFileSync(keyPath),
    cert: readFileSync(certPath),
    allowHTTP1: true,
    ALPNProtocols: ['h2', 'http/1.1'],
  })

  server.on('stream', (stream, headers) => {
    const method = headers[':method']
    const path = String(headers[':path'] ?? '')

    const handle = (buf) => {
      const decoded = namesOf(buf)
      if (!decoded) {
        stream.respond({ ':status': 400 })
        stream.end()
        return
      }
      // Log every queried name so an external test (CI system e2e) can confirm a
      // `dig`/`nslookup` through the redirected system resolver actually reached
      // this mock. One `QUERY=<name>` line per question.
      for (const nm of decoded.names) process.stdout.write(`QUERY=${nm}\n`)
      const answer = dnsPacket.encode({
        type: 'response',
        id: decoded.id,
        flags: dnsPacket.RECURSION_DESIRED | dnsPacket.RECURSION_AVAILABLE,
        questions: decoded.questions,
        answers: decoded.questions.map((q) => ({ type: 'A', name: q.name, ttl: 60, data: '127.0.0.1' })),
      })
      stream.respond({
        ':status': 200,
        'content-type': 'application/dns-message',
        'content-length': answer.length,
      })
      stream.end(answer)
    }

    if (method === 'GET') {
      const m = /[?&]dns=([^&]+)/.exec(path)
      if (!m) {
        stream.respond({ ':status': 400 })
        stream.end()
        return
      }
      handle(Buffer.from(m[1], 'base64url'))
    } else if (method === 'POST') {
      const chunks = []
      stream.on('data', (c) => chunks.push(c))
      stream.on('end', () => handle(Buffer.concat(chunks)))
    } else {
      stream.respond({ ':status': 405 })
      stream.end()
    }
  })

  server.on('error', (err) => {
    process.stderr.write(`mock-doh error: ${err.message}\n`)
    process.exit(1)
  })

  // A failed TLS handshake (e.g. the DoH client rejecting our cert) is otherwise
  // silent. Surface it so the e2e can tell "client never connected" apart from
  // "client connected but distrusted the cert".
  server.on('tlsClientError', (err, sock) => {
    process.stderr.write(`tlsClientError from ${sock.remoteAddress}: ${err.message}\n`)
  })
  server.on('clientError', (err, sock) => {
    process.stderr.write(`clientError: ${err.message}\n`)
    try {
      sock.destroy()
    } catch {}
  })

  server.listen(port, '127.0.0.1', () => {
    const addr = server.address()
    process.stdout.write(`PORT=${addr.port}\n`)
  })
}

const [, , certPath, keyPath, portArg] = process.argv
if (!certPath || !keyPath) {
  process.stderr.write('usage: node mock-doh.mjs <certPath> <keyPath> [port]\n')
  process.exit(2)
}
start(certPath, keyPath, Number(portArg ?? 0) || 0)
