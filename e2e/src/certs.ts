// Mint a throwaway CA + leaf certificate for the mock DoH server.
//
// The proxy validates the upstream DoH TLS connection against an offline trust
// store (webpki roots + an optional extra CA, see voidns-core/src/proxy.rs).
// We add our CA to that extra-CA slot, so the *installed* service trusts the
// mock without any code change. The leaf is presented by the mock HTTPS server;
// its SAN must match the SNI the proxy uses (the `hostname` of the custom
// upstream), so we put `mock.voidns.test` (and 127.0.0.1) in the SAN.

import forge from "node-forge";

export const MOCK_DOH_HOST = "mock.voidns.test";

export interface MintedCerts {
  /** PEM of the CA cert — write this to the service's extra-CA path. */
  caPem: string;
  /** PEM of the leaf cert — served by the mock HTTPS server. */
  certPem: string;
  /** PEM of the leaf private key. */
  keyPem: string;
}

function year(from: Date, n: number): Date {
  const d = new Date(from);
  d.setFullYear(d.getFullYear() + n);
  return d;
}

export function mintCerts(host = MOCK_DOH_HOST): MintedCerts {
  const pki = forge.pki;
  const now = new Date();

  // --- CA ---
  const caKeys = pki.rsa.generateKeyPair(2048);
  const caCert = pki.createCertificate();
  caCert.publicKey = caKeys.publicKey;
  caCert.serialNumber = "01";
  caCert.validity.notBefore = now;
  caCert.validity.notAfter = year(now, 1);
  const caSubject = [{ name: "commonName", value: "VoidNS E2E Test CA" }];
  caCert.setSubject(caSubject);
  caCert.setIssuer(caSubject);
  caCert.setExtensions([
    { name: "basicConstraints", cA: true },
    { name: "keyUsage", keyCertSign: true, cRLSign: true, digitalSignature: true },
  ]);
  caCert.sign(caKeys.privateKey, forge.md.sha256.create());

  // --- leaf (server cert for the mock) signed by the CA ---
  const leafKeys = pki.rsa.generateKeyPair(2048);
  const leaf = pki.createCertificate();
  leaf.publicKey = leafKeys.publicKey;
  leaf.serialNumber = "02";
  leaf.validity.notBefore = now;
  leaf.validity.notAfter = year(now, 1);
  leaf.setSubject([{ name: "commonName", value: host }]);
  leaf.setIssuer(caSubject);
  leaf.setExtensions([
    { name: "basicConstraints", cA: false },
    { name: "keyUsage", digitalSignature: true, keyEncipherment: true },
    { name: "extKeyUsage", serverAuth: true },
    {
      name: "subjectAltName",
      altNames: [
        { type: 2, value: host }, // dNSName
        { type: 7, ip: "127.0.0.1" }, // iPAddress
      ],
    },
  ]);
  leaf.sign(caKeys.privateKey, forge.md.sha256.create());

  return {
    caPem: pki.certificateToPem(caCert),
    certPem: pki.certificateToPem(leaf),
    keyPem: pki.privateKeyToPem(leafKeys.privateKey),
  };
}
