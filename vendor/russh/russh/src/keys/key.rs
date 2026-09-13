// Copyright 2016 Pierre-Étienne Meunier
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
use ssh_encoding::{Decode, Encode};
use ssh_key::public::KeyData;
use ssh_key::{Algorithm, EcdsaCurve, PublicKey};

use crate::keys::Error;

pub trait PublicKeyExt {
    fn decode(bytes: &[u8]) -> Result<PublicKey, Error>;
}

impl PublicKeyExt for PublicKey {
    fn decode(mut bytes: &[u8]) -> Result<PublicKey, Error> {
        let key = KeyData::decode(&mut bytes)?;
        Ok(PublicKey::new(key, ""))
    }
}

#[doc(hidden)]
pub trait Verify {
    fn verify_client_auth(&self, buffer: &[u8], sig: &[u8]) -> bool;
    fn verify_server_auth(&self, buffer: &[u8], sig: &[u8]) -> bool;
}

/// Parse a public key or OpenSSH certificate blob from a byte slice.
/// Teleport reverse-tunnel KEX sends a host *certificate*. Reconstruct a
/// regular public-key blob from the inner key so KEX signature verify works
/// without parsing Teleport's far-future cert timestamps.
pub fn parse_public_key(mut p: &[u8]) -> Result<PublicKey, Error> {
    let mut peek = p;
    if let Ok(algo) = String::decode(&mut peek) {
        if algo.contains("cert-v01") {
            let _nonce = Vec::<u8>::decode(&mut peek)?;
            let inner_algo = if algo.contains("ed25519") {
                "ssh-ed25519"
            } else if algo.contains("nistp256") {
                "ecdsa-sha2-nistp256"
            } else if algo.contains("nistp384") {
                "ecdsa-sha2-nistp384"
            } else if algo.contains("nistp521") {
                "ecdsa-sha2-nistp521"
            } else {
                "ssh-rsa"
            };
            let mut rebuilt = Vec::new();
            inner_algo
                .encode(&mut rebuilt)
                .map_err(|_| Error::CouldNotReadKey)?;
            if inner_algo == "ssh-ed25519" {
                let key = Vec::<u8>::decode(&mut peek).map_err(|_| Error::CouldNotReadKey)?;
                key.encode(&mut rebuilt).map_err(|_| Error::CouldNotReadKey)?;
            } else if inner_algo.starts_with("ecdsa-") {
                let ident = String::decode(&mut peek).map_err(|_| Error::CouldNotReadKey)?;
                let point = Vec::<u8>::decode(&mut peek).map_err(|_| Error::CouldNotReadKey)?;
                ident.encode(&mut rebuilt).map_err(|_| Error::CouldNotReadKey)?;
                point.encode(&mut rebuilt).map_err(|_| Error::CouldNotReadKey)?;
            } else {
                // RSA: e, n mpints
                let e = Vec::<u8>::decode(&mut peek).map_err(|_| Error::CouldNotReadKey)?;
                let n = Vec::<u8>::decode(&mut peek).map_err(|_| Error::CouldNotReadKey)?;
                e.encode(&mut rebuilt).map_err(|_| Error::CouldNotReadKey)?;
                n.encode(&mut rebuilt).map_err(|_| Error::CouldNotReadKey)?;
            }
            let mut r = rebuilt.as_slice();
            return Ok(KeyData::decode(&mut r)?.into());
        }
    }
    Ok(KeyData::decode(&mut p)?.into())
}

/// Obtain a cryptographic-safe random number generator.
pub fn safe_rng() -> impl rand::CryptoRng + rand::RngCore {
    rand::thread_rng()
}

mod private_key_with_hash_alg {
    use std::ops::Deref;
    use std::sync::Arc;

    use ssh_key::Algorithm;

    use crate::helpers::AlgorithmExt;

    /// Helper structure to correlate a key and (in case of RSA) a hash algorithm.
    /// Only used for authentication, not key storage as RSA keys do not inherently
    /// have a hash algorithm associated with them.
    #[derive(Clone, Debug)]
    pub struct PrivateKeyWithHashAlg {
        key: Arc<crate::keys::PrivateKey>,
        hash_alg: Option<crate::keys::HashAlg>,
    }

    impl PrivateKeyWithHashAlg {
        /// Direct constructor.
        ///
        /// For RSA, passing `None` is mapped to the legacy `sha-rsa` (SHA-1).
        /// For other keys, `hash_alg` is ignored.
        pub fn new(
            key: Arc<crate::keys::PrivateKey>,
            mut hash_alg: Option<crate::keys::HashAlg>,
        ) -> Self {
            if !key.algorithm().is_rsa() {
                hash_alg = None;
            }
            Self { key, hash_alg }
        }

        pub fn algorithm(&self) -> Algorithm {
            self.key.algorithm().with_hash_alg(self.hash_alg)
        }

        pub fn hash_alg(&self) -> Option<crate::keys::HashAlg> {
            self.hash_alg
        }
    }

    impl Deref for PrivateKeyWithHashAlg {
        type Target = crate::keys::PrivateKey;

        fn deref(&self) -> &Self::Target {
            &self.key
        }
    }
}

pub use private_key_with_hash_alg::PrivateKeyWithHashAlg;

pub const ALL_KEY_TYPES: &[Algorithm] = &[
    Algorithm::Dsa,
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP384,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP521,
    },
    Algorithm::Ed25519,
    #[cfg(feature = "rsa")]
    Algorithm::Rsa { hash: None },
    #[cfg(feature = "rsa")]
    Algorithm::Rsa {
        hash: Some(ssh_key::HashAlg::Sha256),
    },
    #[cfg(feature = "rsa")]
    Algorithm::Rsa {
        hash: Some(ssh_key::HashAlg::Sha512),
    },
    Algorithm::SkEcdsaSha2NistP256,
    Algorithm::SkEd25519,
];
