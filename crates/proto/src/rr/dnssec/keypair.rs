// Copyright 2015-2016 Benjamin Fry <benjaminfry@me.com>
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// https://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

#[cfg(feature = "dnssec-openssl")]
use std::iter;

#[cfg(feature = "dnssec-openssl")]
use openssl::ec::{EcGroup, EcKey};
#[cfg(feature = "dnssec-openssl")]
use openssl::nid::Nid;
#[cfg(feature = "dnssec-openssl")]
use openssl::pkey::{PKey, Private};
#[cfg(feature = "dnssec-openssl")]
use openssl::rsa::Rsa as OpenSslRsa;
#[cfg(feature = "dnssec-openssl")]
use openssl::symm::Cipher;
#[cfg(feature = "dnssec-ring")]
use ring::{
    rand::{self, SystemRandom},
    signature::{
        EcdsaKeyPair, Ed25519KeyPair, KeyPair as RingKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING,
        ECDSA_P384_SHA384_FIXED_SIGNING,
    },
};

use super::KeyFormat;
use crate::error::{DnsSecErrorKind, DnsSecResult};
#[cfg(feature = "dnssec-openssl")]
use crate::rr::dnssec::DigestType;
use crate::rr::dnssec::{Algorithm, PublicKeyBuf, TBS};

/// Decode private key
#[allow(unused, clippy::match_single_binding)]
pub fn decode_key(
    bytes: &[u8],
    password: Option<&str>,
    algorithm: Algorithm,
    format: KeyFormat,
) -> DnsSecResult<Box<dyn SigningKey>> {
    //  empty string prevents openssl from triggering a read from stdin...

    #[allow(deprecated)]
    match algorithm {
        Algorithm::Unknown(v) => Err(format!("unknown algorithm: {v}").into()),
        #[cfg(feature = "dnssec-openssl")]
        e @ Algorithm::RSASHA1 | e @ Algorithm::RSASHA1NSEC3SHA1 => {
            Err(format!("unsupported Algorithm (insecure): {e:?}").into())
        }
        #[cfg(feature = "dnssec-openssl")]
        Algorithm::RSASHA256 | Algorithm::RSASHA512 => Ok(Box::new(
            RsaSigningKey::decode_key(bytes, password, algorithm, format)
                .map_err(|e| format!("could not translate RSA to KeyPair: {e}"))?,
        )),
        Algorithm::ECDSAP256SHA256 | Algorithm::ECDSAP384SHA384 => match format {
            #[cfg(feature = "dnssec-openssl")]
            KeyFormat::Der | KeyFormat::Pem => Ok(Box::new(EcSigningKey::decode_key(
                bytes, password, algorithm, format,
            )?)),
            #[cfg(feature = "dnssec-ring")]
            KeyFormat::Pkcs8 => Ok(Box::new(EcdsaSigningKey::from_pkcs8(bytes, algorithm)?)),
            e => Err(format!("unsupported key format with EC: {e:?}").into()),
        },
        Algorithm::ED25519 => match format {
            #[cfg(feature = "dnssec-ring")]
            KeyFormat::Pkcs8 => Ok(Box::new(Ed25519SigningKey::from_pkcs8(bytes)?)),
            e => Err(
                format!("unsupported key format with ED25519 (only Pkcs8 supported): {e:?}").into(),
            ),
        },
        e => Err(format!("unsupported Algorithm, enable openssl or ring feature: {e:?}").into()),
    }
}

/// An RSA signing key pair (backed by OpenSSL).
#[cfg(feature = "dnssec-openssl")]
pub struct RsaSigningKey {
    inner: PKey<Private>,
    algorithm: DigestType,
}

#[cfg(feature = "dnssec-openssl")]
impl RsaSigningKey {
    /// Generates a 2048-bits RSA key pair.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::RSASHA256`]
    /// - [`Algorithm::RSASHA512`]
    pub fn generate(algorithm: Algorithm) -> DnsSecResult<Self> {
        match algorithm {
            #[allow(deprecated)]
            Algorithm::RSASHA1 | Algorithm::RSASHA1NSEC3SHA1 => {
                Err("unsupported Algorithm (insecure): {algorithm:?}".into())
            }
            Algorithm::RSASHA256 | Algorithm::RSASHA512 => {
                Self::from_rsa(OpenSslRsa::generate(2_048)?, algorithm)
            }
            _ => Err("invalid Algorithm for RSA key generation: {algorithm:?}".into()),
        }
    }

    /// Decode signing key pair from bytes according to the given `format`.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::RSASHA256`]
    /// - [`Algorithm::RSASHA512`]
    pub fn decode_key(
        bytes: &[u8],
        password: Option<&str>,
        algorithm: Algorithm,
        format: KeyFormat,
    ) -> DnsSecResult<Self> {
        match algorithm {
            #[allow(deprecated)]
            Algorithm::RSASHA1 | Algorithm::RSASHA1NSEC3SHA1 => {
                return Err(format!("unsupported Algorithm (insecure): {algorithm:?}").into())
            }
            Algorithm::RSASHA256 | Algorithm::RSASHA512 => {}
            _ => {
                return Err(format!("invalid Algorithm for RSA: {algorithm:?}").into());
            }
        }

        let key = match format {
            KeyFormat::Der => OpenSslRsa::private_key_from_der(bytes)
                .map_err(|e| format!("error reading RSA as DER: {e}"))?,
            KeyFormat::Pem => {
                //  empty string prevents openssl from triggering a read from stdin...
                let password = password.unwrap_or("");
                OpenSslRsa::private_key_from_pem_passphrase(bytes, password.as_bytes())
                    .map_err(|e| format!("could not decode RSA from PEM, bad password?: {e}"))?
            }
            _ => {
                return Err(format!(
                    "unsupported key format with RSA (DER or PEM only): {format:?}"
                )
                .into())
            }
        };

        Ok(Self::from_rsa(key, algorithm)
            .map_err(|e| format!("could not decode RSA key pair: {e}"))?)
    }

    /// Creates an RSA type key pair.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::RSASHA256`]
    /// - [`Algorithm::RSASHA512`]
    pub fn from_rsa(rsa: OpenSslRsa<Private>, algorithm: Algorithm) -> DnsSecResult<Self> {
        Self::from_rsa_pkey(PKey::from_rsa(rsa)?, algorithm)
    }

    /// Creates an RSA type key pair.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::RSASHA256`]
    /// - [`Algorithm::RSASHA512`]
    pub fn from_rsa_pkey(inner: PKey<Private>, algorithm: Algorithm) -> DnsSecResult<Self> {
        match algorithm {
            #[allow(deprecated)]
            Algorithm::RSASHA1 | Algorithm::RSASHA1NSEC3SHA1 => {
                Err(format!("unsupported signing algorithm (insecure): {algorithm:?}").into())
            }
            Algorithm::RSASHA256 | Algorithm::RSASHA512 => Ok(Self {
                inner,
                algorithm: DigestType::from(algorithm),
            }),
            _ => {
                Err(DnsSecErrorKind::Message("unsupported signing algorithm: {algorithm:?}").into())
            }
        }
    }

    /// Encode the key pair to DER-encoded ASN.1 bytes.
    pub fn encode_der(&self) -> DnsSecResult<Vec<u8>> {
        self.inner
            .private_key_to_der()
            .map_err(|e| format!("error writing key as DER: {e}").into())
    }

    /// Encode the key pair to DER-encoded ASN.1 bytes, optionally encrypted with `password`.
    pub fn encode_pem(&self, password: Option<&str>) -> DnsSecResult<Vec<u8>> {
        if let Some(password) = password {
            self.inner
                .private_key_to_pem_pkcs8_passphrase(Cipher::aes_256_cbc(), password.as_bytes())
        } else {
            self.inner.private_key_to_pem_pkcs8()
        }
        .map_err(|e| format!("error writing key as PEM: {e}").into())
    }
}

#[cfg(feature = "dnssec-openssl")]
impl SigningKey for RsaSigningKey {
    fn sign(&self, tbs: &TBS) -> DnsSecResult<Vec<u8>> {
        let digest = self.algorithm.to_openssl_digest()?;
        let mut signer = openssl::sign::Signer::new(digest, &self.inner)?;
        signer.update(tbs.as_ref())?;
        Ok(signer.sign_to_vec()?)
    }

    fn to_public_key(&self) -> DnsSecResult<PublicKeyBuf> {
        let rsa = self.inner.rsa()?;
        Ok(PublicKeyBuf::from_rsa(&rsa))
    }
}

/// An ECDSA signing key pair (backed by OpenSSL).
#[cfg(feature = "dnssec-openssl")]
pub struct EcSigningKey {
    inner: PKey<Private>,
    algorithm: DigestType,
}

#[cfg(feature = "dnssec-openssl")]
impl EcSigningKey {
    /// Generates a 2048-bits RSA key pair.
    pub fn generate(algorithm: Algorithm) -> DnsSecResult<Self> {
        match algorithm {
            #[cfg(feature = "dnssec-openssl")]
            Algorithm::ECDSAP256SHA256 => {
                let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
                let inner = EcKey::generate(&group)?;
                Self::from_ec_key(inner, algorithm)
            }
            #[cfg(feature = "dnssec-openssl")]
            Algorithm::ECDSAP384SHA384 => {
                let group = EcGroup::from_curve_name(Nid::SECP384R1)?;
                let inner = EcKey::generate(&group)?;
                Self::from_ec_key(inner, algorithm)
            }
            _ => {
                Err(format!("unsupported Algorithm for ECDSA key generation: {algorithm:?}").into())
            }
        }
    }

    /// Decode signing key pair from bytes according to the given `format`.
    pub fn decode_key(
        bytes: &[u8],
        password: Option<&str>,
        algorithm: Algorithm,
        format: KeyFormat,
    ) -> DnsSecResult<Self> {
        match algorithm {
            Algorithm::ECDSAP256SHA256 | Algorithm::ECDSAP384SHA384 => {}
            _ => {
                return Err(format!("invalid Algorithm for EcSigningKey: {algorithm:?}").into());
            }
        }

        let key = match format {
            KeyFormat::Der => EcKey::private_key_from_der(bytes)
                .map_err(|e| format!("error reading EC key as DER: {e}"))?,
            KeyFormat::Pem => {
                //  empty string prevents openssl from triggering a read from stdin...
                let password = password.unwrap_or("");
                EcKey::private_key_from_pem_passphrase(bytes, password.as_bytes())
                    .map_err(|e| format!("could not decode EC key from PEM, bad password?: {e}"))?
            }
            _ => {
                return Err(format!(
                    "unsupported key format with EC key (DER or PEM only): {format:?}"
                )
                .into())
            }
        };

        Ok(Self::from_ec_key(key, algorithm)
            .map_err(|e| format!("could not decode EC key: {e}"))?)
    }

    /// Creates an elliptic curve key pair.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::ECDSAP256SHA256`]
    /// - [`Algorithm::ECDSAP384SHA384`]
    pub fn from_ec_key(ec_key: EcKey<Private>, algorithm: Algorithm) -> DnsSecResult<Self> {
        Self::from_ec_pkey(PKey::from_ec_key(ec_key)?, algorithm)
    }

    /// Given a known pkey of an RSA key, return the wrapped key pair.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::ECDSAP256SHA256`]
    /// - [`Algorithm::ECDSAP384SHA384`]
    pub fn from_ec_pkey(inner: PKey<Private>, algorithm: Algorithm) -> DnsSecResult<Self> {
        match algorithm {
            Algorithm::ECDSAP256SHA256 | Algorithm::ECDSAP384SHA384 => Ok(Self {
                inner,
                algorithm: DigestType::from(algorithm),
            }),
            _ => Err(DnsSecErrorKind::Message("unsupported signing algorithm").into()),
        }
    }

    /// Encode the key pair to DER-encoded ASN.1 bytes.
    pub fn encode_der(&self) -> DnsSecResult<Vec<u8>> {
        self.inner
            .private_key_to_der()
            .map_err(|e| format!("error writing key as DER: {e}").into())
    }

    /// Encode the key pair to DER-encoded ASN.1 bytes, optionally encrypted with `password`.
    pub fn encode_pem(&self, password: Option<&str>) -> DnsSecResult<Vec<u8>> {
        if let Some(password) = password {
            self.inner
                .private_key_to_pem_pkcs8_passphrase(Cipher::aes_256_cbc(), password.as_bytes())
        } else {
            self.inner.private_key_to_pem_pkcs8()
        }
        .map_err(|e| format!("error writing key as PEM: {e}").into())
    }
}

#[cfg(feature = "dnssec-openssl")]
impl SigningKey for EcSigningKey {
    fn sign(&self, tbs: &TBS) -> DnsSecResult<Vec<u8>> {
        let digest = self.algorithm.to_openssl_digest()?;
        let mut signer = openssl::sign::Signer::new(digest, &self.inner)?;
        signer.update(tbs.as_ref())?;
        let bytes = signer.sign_to_vec()?;

        // Convert DER signature to raw signature (see RFC 6605 Section 4)
        if bytes.len() < 8 {
            return Err("unexpected signature format (length too short)".into());
        }
        let expect = |pos: usize, expected: u8| -> DnsSecResult<()> {
            if bytes[pos] != expected {
                return Err(format!("unexpected signature format ({pos}, {expected}))").into());
            }
            Ok(())
        };
        // Sanity checks
        expect(0, 0x30)?;
        expect(1, (bytes.len() - 2) as u8)?;
        expect(2, 0x02)?;
        let p1_len = bytes[3] as usize;
        let p2_pos = 4 + p1_len;
        expect(p2_pos, 0x02)?;
        let p2_len = bytes[p2_pos + 1] as usize;
        if p2_pos + 2 + p2_len > bytes.len() {
            return Err("unexpected signature format (invalid length)".into());
        }

        let p1 = &bytes[4..p2_pos];
        let p2 = &bytes[p2_pos + 2..p2_pos + 2 + p2_len];

        // For P-256, each integer MUST be encoded as 32 octets;
        // for P-384, each integer MUST be encoded as 48 octets.
        let part_len = match self.algorithm {
            DigestType::SHA256 => 32,
            DigestType::SHA384 => 48,
            _ => return Err("unexpected algorithm".into()),
        };

        let mut ret = Vec::<u8>::new();
        {
            let mut write_part = |mut part: &[u8]| -> DnsSecResult<()> {
                // We need to pad or trim the octet string to expected length
                if part.len() > part_len + 1 {
                    return Err("invalid signature data".into());
                }
                if part.len() == part_len + 1 {
                    // Trim leading zero
                    if part[0] != 0x00 {
                        return Err("invalid signature data".into());
                    }
                    part = &part[1..];
                }

                // Pad with zeros. All numbers are big-endian here.
                ret.extend(iter::repeat(0x00).take(part_len - part.len()));
                ret.extend(part);
                Ok(())
            };
            write_part(p1)?;
            write_part(p2)?;
        }
        assert_eq!(ret.len(), part_len * 2);
        Ok(ret)
    }

    fn to_public_key(&self) -> DnsSecResult<PublicKeyBuf> {
        let ec = self.inner.ec_key()?;
        PublicKeyBuf::from_ec(&ec)
    }
}

/// An ECDSA signing key pair (backed by ring).
#[cfg(feature = "dnssec-ring")]
pub struct EcdsaSigningKey {
    inner: EcdsaKeyPair,
}

#[cfg(feature = "dnssec-ring")]
impl EcdsaSigningKey {
    /// Decode signing key pair from DER-encoded PKCS#8 bytes.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::ECDSAP256SHA256`]
    /// - [`Algorithm::ECDSAP384SHA384`]
    pub fn from_pkcs8(bytes: &[u8], algorithm: Algorithm) -> DnsSecResult<Self> {
        let rng = SystemRandom::new();
        let ring_algorithm = if algorithm == Algorithm::ECDSAP256SHA256 {
            &ECDSA_P256_SHA256_FIXED_SIGNING
        } else if algorithm == Algorithm::ECDSAP384SHA384 {
            &ECDSA_P384_SHA384_FIXED_SIGNING
        } else {
            return Err(DnsSecErrorKind::Message("unsupported algorithm").into());
        };

        Ok(Self {
            inner: EcdsaKeyPair::from_pkcs8(ring_algorithm, bytes, &rng)?,
        })
    }

    /// Creates an ECDSA key pair with ring.
    pub fn from_ecdsa(inner: EcdsaKeyPair) -> Self {
        Self { inner }
    }

    /// Generate signing key pair and return the DER-encoded PKCS#8 bytes.
    ///
    /// Errors unless the given algorithm is one of the following:
    ///
    /// - [`Algorithm::ECDSAP256SHA256`]
    /// - [`Algorithm::ECDSAP384SHA384`]
    pub fn generate_pkcs8(algorithm: Algorithm) -> DnsSecResult<Vec<u8>> {
        let rng = SystemRandom::new();
        let alg = if algorithm == Algorithm::ECDSAP256SHA256 {
            &ECDSA_P256_SHA256_FIXED_SIGNING
        } else if algorithm == Algorithm::ECDSAP384SHA384 {
            &ECDSA_P384_SHA384_FIXED_SIGNING
        } else {
            return Err(DnsSecErrorKind::Message("unsupported algorithm").into());
        };

        let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, &rng)?;
        Ok(pkcs8.as_ref().to_vec())
    }
}

#[cfg(feature = "dnssec-ring")]
impl SigningKey for EcdsaSigningKey {
    fn sign(&self, tbs: &TBS) -> DnsSecResult<Vec<u8>> {
        let rng = rand::SystemRandom::new();
        Ok(self.inner.sign(&rng, tbs.as_ref())?.as_ref().to_vec())
    }

    fn to_public_key(&self) -> DnsSecResult<PublicKeyBuf> {
        let mut bytes = self.inner.public_key().as_ref().to_vec();
        bytes.remove(0);
        Ok(PublicKeyBuf::new(bytes))
    }
}

/// An Ed25519 signing key pair (backed by ring).
#[cfg(feature = "dnssec-ring")]
pub struct Ed25519SigningKey {
    inner: Ed25519KeyPair,
}

#[cfg(feature = "dnssec-ring")]
impl Ed25519SigningKey {
    /// Decode signing key pair from DER-encoded PKCS#8 bytes.
    pub fn from_pkcs8(bytes: &[u8]) -> DnsSecResult<Self> {
        Ok(Self {
            inner: Ed25519KeyPair::from_pkcs8(bytes)?,
        })
    }

    /// Creates an Ed25519 keypair.
    pub fn from_ed25519(inner: Ed25519KeyPair) -> Self {
        Self { inner }
    }

    /// Generate signing key pair and return the DER-encoded PKCS#8 bytes.
    pub fn generate_pkcs8() -> DnsSecResult<Vec<u8>> {
        let rng = rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)?;
        Ok(pkcs8.as_ref().to_vec())
    }
}

#[cfg(feature = "dnssec-ring")]
impl SigningKey for Ed25519SigningKey {
    fn sign(&self, tbs: &TBS) -> DnsSecResult<Vec<u8>> {
        Ok(self.inner.sign(tbs.as_ref()).as_ref().to_vec())
    }

    fn to_public_key(&self) -> DnsSecResult<PublicKeyBuf> {
        Ok(PublicKeyBuf::new(self.inner.public_key().as_ref().to_vec()))
    }
}

/// A key that can be used to sign records.
pub trait SigningKey: Send + Sync + 'static {
    /// Sign DNS records.
    ///
    /// # Return value
    ///
    /// The signature, ready to be stored in an `RData::RRSIG`.
    fn sign(&self, tbs: &TBS) -> DnsSecResult<Vec<u8>>;

    /// Returns a [`PublicKeyBuf`] for this [`SigningKey`].
    fn to_public_key(&self) -> DnsSecResult<PublicKeyBuf>;
}

#[cfg(any(feature = "dnssec-openssl", feature = "dnssec-ring"))]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rr::dnssec::{PublicKey, Verifier};

    #[cfg(feature = "dnssec-openssl")]
    #[test]
    fn test_rsa() {
        let algorithm = Algorithm::RSASHA256;
        let key = RsaSigningKey::generate(algorithm).unwrap();
        public_key_test(&key, algorithm);

        let neg = RsaSigningKey::generate(algorithm).unwrap();
        hash_test(&key, &neg, algorithm);
    }

    #[cfg(feature = "dnssec-openssl")]
    #[test]
    fn test_ec_p256() {
        let algorithm = Algorithm::ECDSAP256SHA256;
        let key = EcSigningKey::generate(algorithm).unwrap();
        public_key_test(&key, algorithm);

        let neg = EcSigningKey::generate(algorithm).unwrap();
        hash_test(&key, &neg, algorithm);
    }

    #[cfg(feature = "dnssec-ring")]
    #[test]
    fn test_ec_p256_pkcs8() {
        let algorithm = Algorithm::ECDSAP256SHA256;
        let format = KeyFormat::Pkcs8;
        let pkcs8 = EcdsaSigningKey::generate_pkcs8(algorithm).unwrap();
        let key = decode_key(&pkcs8, None, algorithm, format).unwrap();
        public_key_test(&*key, algorithm);

        let neg_pkcs8 = EcdsaSigningKey::generate_pkcs8(algorithm).unwrap();
        let neg = decode_key(&neg_pkcs8, None, algorithm, format).unwrap();
        hash_test(&*key, &*neg, algorithm);
    }

    #[cfg(feature = "dnssec-openssl")]
    #[test]
    fn test_ec_p384() {
        let algorithm = Algorithm::ECDSAP384SHA384;
        let key = EcSigningKey::generate(algorithm).unwrap();
        public_key_test(&key, algorithm);

        let neg = EcSigningKey::generate(algorithm).unwrap();
        hash_test(&key, &neg, algorithm);
    }

    #[cfg(feature = "dnssec-ring")]
    #[test]
    fn test_ec_p384_pkcs8() {
        let algorithm = Algorithm::ECDSAP384SHA384;
        let format = KeyFormat::Pkcs8;
        let pkcs8 = EcdsaSigningKey::generate_pkcs8(algorithm).unwrap();
        let key = decode_key(&pkcs8, None, algorithm, format).unwrap();
        public_key_test(&*key, algorithm);

        let neg_pkcs8 = EcdsaSigningKey::generate_pkcs8(algorithm).unwrap();
        let neg = decode_key(&neg_pkcs8, None, algorithm, format).unwrap();
        hash_test(&*key, &*neg, algorithm);
    }

    #[cfg(feature = "dnssec-ring")]
    #[test]
    fn test_ed25519() {
        let algorithm = Algorithm::ED25519;
        let format = KeyFormat::Pkcs8;
        let pkcs8 = Ed25519SigningKey::generate_pkcs8().unwrap();
        let key = decode_key(&pkcs8, None, algorithm, format).unwrap();
        public_key_test(&*key, algorithm);

        let neg_pkcs8 = Ed25519SigningKey::generate_pkcs8().unwrap();
        let neg = decode_key(&neg_pkcs8, None, algorithm, format).unwrap();
        hash_test(&*key, &*neg, algorithm);
    }

    fn public_key_test(key: &dyn SigningKey, algorithm: Algorithm) {
        let pk = key.to_public_key().unwrap();

        let tbs = TBS::from(&b"www.example.com"[..]);
        let mut sig = key.sign(&tbs).unwrap();
        assert!(
            pk.verify(algorithm, tbs.as_ref(), &sig).is_ok(),
            "algorithm: {algorithm:?} (public key)",
        );
        sig[10] = !sig[10];
        assert!(
            pk.verify(algorithm, tbs.as_ref(), &sig).is_err(),
            "algorithm: {algorithm:?} (public key, neg)",
        );
    }

    fn hash_test(key: &dyn SigningKey, neg: &dyn SigningKey, algorithm: Algorithm) {
        let tbs = TBS::from(&b"www.example.com"[..]);

        // TODO: convert to stored keys...
        let pub_key = key.to_public_key().unwrap();
        let neg_pub_key = neg.to_public_key().unwrap();

        let sig = key.sign(&tbs).unwrap();
        assert!(
            pub_key.verify(algorithm, tbs.as_ref(), &sig).is_ok(),
            "algorithm: {algorithm:?}",
        );
        assert!(
            key.to_public_key()
                .unwrap()
                .to_dnskey(algorithm)
                .verify(tbs.as_ref(), &sig)
                .is_ok(),
            "algorithm: {algorithm:?} (dnskey)",
        );
        assert!(
            neg_pub_key.verify(algorithm, tbs.as_ref(), &sig).is_err(),
            "algorithm: {:?} (neg)",
            algorithm
        );
        assert!(
            neg.to_public_key()
                .unwrap()
                .to_dnskey(algorithm)
                .verify(tbs.as_ref(), &sig)
                .is_err(),
            "algorithm: {algorithm:?} (dnskey, neg)",
        );
    }
}
