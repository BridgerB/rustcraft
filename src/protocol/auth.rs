//! Server-join cryptography: Minecraft's authentication hash and RSA
//! public-key encryption of the shared secret / verify token. The HTTP session
//! handshake (`join_server`) lives with the async client/auth layer.

use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use sha1::{Digest, Sha1};

/// Compute Minecraft's authentication server hash:
/// `mcHexDigest(SHA1(serverId + sharedSecret + publicKey))`.
pub fn mc_server_hash(server_id: &str, shared_secret: &[u8], public_key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key);
    let digest = hasher.finalize();
    mc_hex_digest(&digest)
}

/// Format a 20-byte SHA1 digest as Minecraft's signed hex string (two's
/// complement with a leading `-` when the high bit is set).
fn mc_hex_digest(digest: &[u8]) -> String {
    let mut bytes = digest.to_vec();
    let negative = bytes[0] & 0x80 != 0;
    if negative {
        let mut carry = true;
        for b in bytes.iter_mut().rev() {
            *b = !*b;
            if carry {
                carry = *b == 0xff;
                *b = b.wrapping_add(1);
            }
        }
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!("-{}", hex.trim_start_matches('0'))
    } else {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        hex.trim_start_matches('0').to_string()
    }
}

/// RSA-encrypt `data` with a DER-encoded (SubjectPublicKeyInfo) public key
/// using PKCS#1 v1.5 padding — used for the encryption-response packet.
pub fn rsa_public_encrypt(der_public_key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let key = RsaPublicKey::from_public_key_der(der_public_key)
        .map_err(|e| format!("invalid public key: {e}"))?;
    let mut rng = rand::thread_rng();
    key.encrypt(&mut rng, Pkcs1v15Encrypt, data)
        .map_err(|e| format!("rsa encrypt failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known wiki.vg test vectors: mcHexDigest(SHA1(name)).
    #[test]
    fn server_hash_test_vectors() {
        assert_eq!(
            mc_server_hash("Notch", &[], &[]),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            mc_server_hash("jeb_", &[], &[]),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(
            mc_server_hash("simon", &[], &[]),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn rsa_encrypt_roundtrips() {
        use rsa::pkcs8::EncodePublicKey;
        use rsa::RsaPrivateKey;
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = RsaPublicKey::from(&priv_key);
        let der = pub_key.to_public_key_der().unwrap();

        let secret = [42u8; 16];
        let ciphertext = rsa_public_encrypt(der.as_bytes(), &secret).unwrap();
        let decrypted = priv_key.decrypt(Pkcs1v15Encrypt, &ciphertext).unwrap();
        assert_eq!(decrypted, secret);
    }
}
