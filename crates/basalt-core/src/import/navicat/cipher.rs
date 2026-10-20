//! Navicat 12 及以后版本导出文件里的密码:AES-128-CBC、PKCS7 填充、十六进制编码。
//! 密钥和 IV 是 Navicat 写死的公开常量,所以 .ncx 文件本身就能还原出密码。

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, KeyIvInit};

type Decryptor = cbc::Decryptor<aes::Aes128>;

const KEY: &[u8; 16] = b"libcckeylibcckey";
const IV: &[u8; 16] = b"libcciv libcciv ";
const BLOCK_BYTES: usize = 16;

/// 解不开(不是十六进制、长度不对、填充不对、不是 UTF-8)一律返回 `None`,
/// 由调用方标成「需首次打开时输入密码」,不中断整个导入。
pub fn decrypt_password(hex_text: &str) -> Option<String> {
    let bytes = hex::decode(hex_text.trim()).ok()?;
    if bytes.is_empty() || bytes.len() % BLOCK_BYTES != 0 {
        return None;
    }
    let plain = Decryptor::new(KEY.into(), IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
        .ok()?;
    let text = String::from_utf8(plain).ok()?;
    // 用错算法解出来的碰巧也可能填充合法,带控制字符的一定不是真密码。
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(text)
}

#[cfg(test)]
pub(crate) fn encrypt_password(plain: &str) -> String {
    use aes::cipher::BlockEncryptMut;
    let cipher = cbc::Encryptor::<aes::Aes128>::new(KEY.into(), IV.into())
        .encrypt_padded_vec_mut::<Pkcs7>(plain.as_bytes());
    hex::encode_upper(cipher)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用 openssl(独立实现)按同样的密钥和 IV 加密假密码得到的对照值:
    /// `printf '%s' "$p" | openssl enc -aes-128-cbc -K <hex(KEY)> -iv <hex(IV)> | xxd -p -u`
    const OPENSSL_VECTORS: [(&str, &str); 4] = [
        ("Fake-Pass#2026", "7CD6F821FD5487C86ADFB4EE5FA0C277"),
        (
            "sixteen-chars-pw",
            "ABD6C2C1F9660EDA68A2204E976ACE44B43D515B635210328EAA2D5310FCC5F3",
        ),
        ("测试密码", "EEA1D394AF85EE94E0540D99DDF4251A"),
        ("a", "5CD8295C9776753E314CFC428A353BF4"),
    ];

    #[test]
    fn decrypts_values_produced_by_an_independent_implementation() {
        for (plain, cipher) in OPENSSL_VECTORS {
            assert_eq!(decrypt_password(cipher).as_deref(), Some(plain));
            assert_eq!(
                decrypt_password(&cipher.to_lowercase()).as_deref(),
                Some(plain),
                "十六进制大小写都应接受"
            );
        }
    }

    #[test]
    fn test_encryptor_matches_the_same_vectors() {
        for (plain, cipher) in OPENSSL_VECTORS {
            assert_eq!(encrypt_password(plain), cipher);
        }
    }

    #[test]
    fn rejects_values_that_are_not_navicat12_ciphertext() {
        let bad = [
            "",
            "not hex at all!",
            "ABC",              // 奇数位
            "0123456789ABCDEF", // 8 字节:老版本 Blowfish 的块长,不是 AES 块的整数倍
            // 16 字节但填充不合法
            "00000000000000000000000000000000",
        ];
        for value in bad {
            assert_eq!(decrypt_password(value), None, "{value}");
        }
    }

    #[test]
    fn rejects_plaintext_with_control_characters() {
        assert_eq!(decrypt_password(&encrypt_password("a\u{1}b")), None);
    }
}
