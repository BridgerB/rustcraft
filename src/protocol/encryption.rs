//! AES-128-CFB8 stream cipher. Minecraft uses the 16-byte shared secret as both
//! the key and the IV, and runs CFB in 8-bit (byte) feedback mode as a
//! continuous stream across the connection.

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;

/// A stateful AES-128-CFB8 cipher (one instance for encrypt, one for decrypt).
pub struct Cfb8 {
    cipher: Aes128,
    iv: [u8; 16],
    decrypt: bool,
}

impl Cfb8 {
    fn new(secret: &[u8], decrypt: bool) -> Cfb8 {
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&secret[..16]);
        Cfb8 {
            cipher: Aes128::new(secret[..16].into()),
            iv,
            decrypt,
        }
    }

    pub fn encryptor(secret: &[u8]) -> Cfb8 {
        Cfb8::new(secret, false)
    }

    pub fn decryptor(secret: &[u8]) -> Cfb8 {
        Cfb8::new(secret, true)
    }

    /// Process `data` in place, advancing the CFB8 shift register.
    pub fn update(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            let mut block = self.iv;
            self.cipher.encrypt_block((&mut block).into());
            let keystream = block[0];
            let plain_or_cipher_in = *byte;
            let out = plain_or_cipher_in ^ keystream;
            // Shift register: drop oldest byte, append the ciphertext byte.
            let feedback = if self.decrypt {
                plain_or_cipher_in
            } else {
                out
            };
            self.iv.copy_within(1..16, 0);
            self.iv[15] = feedback;
            *byte = out;
        }
    }

    /// Convenience: process a copy and return it.
    pub fn update_vec(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        self.update(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let secret = [7u8; 16];
        let mut enc = Cfb8::encryptor(&secret);
        let mut dec = Cfb8::decryptor(&secret);
        let plaintext = b"the quick brown fox jumps over the lazy dog".to_vec();
        let ciphertext = enc.update_vec(&plaintext);
        assert_ne!(ciphertext, plaintext);
        let decrypted = dec.update_vec(&ciphertext);
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn streaming_matches_oneshot() {
        let secret = [3u8; 16];
        let data = vec![0xabu8; 100];
        let mut a = Cfb8::encryptor(&secret);
        let one_shot = a.update_vec(&data);

        let mut b = Cfb8::encryptor(&secret);
        let mut streamed = Vec::new();
        for chunk in data.chunks(7) {
            streamed.extend(b.update_vec(chunk));
        }
        assert_eq!(one_shot, streamed);
    }
}
