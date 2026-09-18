/**
 * Certificate fixtures shared by the two suites that need a **real** certificate.
 *
 * ## Why real, and why pasted in rather than generated at test time
 *
 * `identifyCert()` parses the DER with `crypto.X509Certificate`, so a placeholder PEM like
 * `-----BEGIN CERTIFICATE-----\nFAKE_CERT\n-----END CERTIFICATE-----` is not a certificate and the
 * function correctly returns `null` for it. The pre-P3 tests used exactly that placeholder, and
 * `generateCA()` now **refuses to report success** on output it cannot parse — so the fixture has to
 * be genuine or the suite is testing the failure path by accident.
 *
 * Calling the real `mkcert createCA()` in a unit test would work but costs a keypair generation and
 * makes the expected fingerprint unknowable at authoring time. A pasted certificate keeps the
 * assertions **literal** — `EXPECTED_FINGERPRINT` is a value computed once, independently, and
 * pinned — which is what makes them regression tests rather than restatements of the code under
 * test.
 *
 * ## Provenance
 *
 * Generated with this repository's own `mkcert` dependency, using the same options
 * `packages/engine/src/proxy/certManager.ts#generateCA` passes:
 *
 * ```js
 * createCA({ organization: "Bifurc CA", countryCode: "US", state: "Development",
 *            locality: "Local", validity: 3650 })
 * ```
 *
 * It is a throwaway development CA. It is **public** by construction — a certificate, never a
 * private key — and the "key" below is a placeholder, because nothing under test reads a private
 * key's contents.
 */

/** A real CA certificate. See the header for its provenance. */
export const REAL_CA_PEM = `-----BEGIN CERTIFICATE-----
MIIDWTCCAkGgAwIBAgIGMTM1OTIwMA0GCSqGSIb3DQEBCwUAMFsxEjAQBgNVBAMT
CUJpZnVyYyBDQTELMAkGA1UEBhMCVVMxFDASBgNVBAgTC0RldmVsb3BtZW50MQ4w
DAYDVQQHEwVMb2NhbDESMBAGA1UEChMJQmlmdXJjIENBMB4XDTI2MDkxNzA2Mzcw
MFoXDTM2MDkxNDA2MzcwMFowWzESMBAGA1UEAxMJQmlmdXJjIENBMQswCQYDVQQG
EwJVUzEUMBIGA1UECBMLRGV2ZWxvcG1lbnQxDjAMBgNVBAcTBUxvY2FsMRIwEAYD
VQQKEwlCaWZ1cmMgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCR
G4f8vHHHIHHn91FFEJr2CBPedVh5186/YGEqk+dfGXz5aX17DRFWsO1bosxqD27D
RMLoZBRIheioDhqHEN70VbD5E+DnAUqi4irS/gc1qrXLqTJuxab1/A3ys2VjA7fv
fR1ujVLMpwf00IUvr4IB6ik9jHPJPSkrYafnSk1KfMyGZcrsc5Qbj+SlvF5GMQiW
zpMsUQsFB8iUKmxetrQCfEVxI4sP2pgs1KebYi1a1Uz2jh6wt0eOPQIQ8XwCwlyl
kkQTJIiWhvERDb/dUJNCYsugXJz1rQfPykY6KukXovdwMRwmRkbc0/qemI6jhSsD
32t/F2n2+D3JSt11K27VAgMBAAGjIzAhMA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0P
AQH/BAQDAgIEMA0GCSqGSIb3DQEBCwUAA4IBAQAU84Qqr2s63t1bJzHYzM2ThXQy
Lb2hUTBnbSqWy6nVZ4GQVyDymJMALjcJ26+BVyGZfcYD8LsoD5HLPinv2yjWudQL
bHtEwwvZnON3oTZsDzhXTYjW6CiGSaHrZXiHPvowexOjAgPHI/ezkszD5r2aUARj
wcQHzFJbadF368sqc+SWaZDsjnxUeWCizeVsEf+wMrNwmJ1ZgCI+jBG3TMY/6JaE
GNar9TzlG54PYdDm/1FzCngDEXRd9FPQmRi+csP6newFAE/p5xw39zwGxEyPjamM
2rIjrwEvIZOw6sTmJ6CaSDIOi4DCRBq1UjqtHQmgew9zSt1lr6oj9nWKqoQg
-----END CERTIFICATE-----
`;

/** SHA-256 of the DER, in the `sha256:<hex>` form the protocol carries. */
export const REAL_CA_FINGERPRINT =
    "sha256:46a047684850965bceef9e36278447ce90c93184f96dc017946bb90103b22d32";

/** SHA-1, upper-case and unseparated — the form `certutil -delstore` and `security -Z` take. */
export const REAL_CA_THUMBPRINT_SHA1 = "CD1FDAFE36008D23A3484F29AE84A6B3EABD9F63";

/** A substring of the subject DN, for assertions that only need "is this the right certificate?". */
export const REAL_CA_SUBJECT_MARKER = "Bifurc CA";

/**
 * A PEM whose body is not DER. Correctly rejected by `identifyCert()`, and the input
 * `generateCA()` must refuse to report success on.
 */
export const FAKE_CA_PEM = "-----BEGIN CERTIFICATE-----\nFAKE_CERT\n-----END CERTIFICATE-----\n";

/** Placeholder key material — nothing under test parses a private key. */
export const REAL_KEY_PEM = "-----BEGIN PRIVATE KEY-----\nPLACEHOLDER_KEY\n-----END PRIVATE KEY-----\n";
