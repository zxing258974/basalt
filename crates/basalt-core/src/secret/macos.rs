//! 不弹框地删除钥匙串条目:直接 SecItemDelete,并要求需要用户授权时立刻失败。

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use security_framework_sys::base::SecCopyErrorMessageString;
use security_framework_sys::item::{
    kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword, kSecUseAuthenticationUI,
};
use security_framework_sys::keychain_item::SecItemDelete;

use super::QuietDelete;
use crate::error::{CoreError, CoreResult};

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    /// security-framework-sys 没有导出它。作为 kSecUseAuthenticationUI 的值:要授权时不弹框,直接报错。
    static kSecUseAuthenticationUIFail: CFStringRef;
}

const ITEM_NOT_FOUND: i32 = -25300;
/// 条目不归当前程序所有,删它等于改它的访问控制。
const INVALID_OWNER_EDIT: i32 = -25244;
const INTERACTION_NOT_ALLOWED: i32 = -25308;
const AUTH_FAILED: i32 = -25293;
const USER_CANCELED: i32 = -128;

pub(super) fn delete_generic_password(service: &str, account: &str) -> CoreResult<QuietDelete> {
    // SAFETY: 都是 Security 框架导出的常量 CFString,整个进程期间有效;按 Get 规则包装不转移所有权。
    let (class, generic, service_key, account_key, ui_key, ui_fail) = unsafe {
        (
            CFString::wrap_under_get_rule(kSecClass),
            CFString::wrap_under_get_rule(kSecClassGenericPassword),
            CFString::wrap_under_get_rule(kSecAttrService),
            CFString::wrap_under_get_rule(kSecAttrAccount),
            CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
            CFString::wrap_under_get_rule(kSecUseAuthenticationUIFail),
        )
    };
    let query: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
        (class, generic.as_CFType()),
        (service_key, CFString::new(service).as_CFType()),
        (account_key, CFString::new(account).as_CFType()),
        (ui_key, ui_fail.as_CFType()),
    ]);
    // SAFETY: query 是有效的 CFDictionary,调用期间一直由这里持有。
    outcome(unsafe { SecItemDelete(query.as_concrete_TypeRef()) })
}

fn outcome(status: i32) -> CoreResult<QuietDelete> {
    match status {
        0 | ITEM_NOT_FOUND => Ok(QuietDelete::Removed),
        INVALID_OWNER_EDIT | INTERACTION_NOT_ALLOWED | AUTH_FAILED | USER_CANCELED => {
            Ok(QuietDelete::NeedsAuthorization)
        }
        other => Err(CoreError::Secret(error_message(other))),
    }
}

fn error_message(status: i32) -> String {
    // SAFETY: 返回值按 Create 规则归调用方;为空表示没有对应的说明。
    let text = unsafe { SecCopyErrorMessageString(status, std::ptr::null_mut()) };
    if text.is_null() {
        return format!("钥匙串返回错误 {status}");
    }
    // SAFETY: 上面确认过非空,所有权交给 CFString,离开作用域时释放。
    let text = unsafe { CFString::wrap_under_create_rule(text) };
    format!("{text}（{status}）")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::secret::{KeychainStore, SecretStore};

    #[test]
    fn statuses_map_to_outcomes() {
        assert_eq!(outcome(0).unwrap(), QuietDelete::Removed);
        assert_eq!(outcome(ITEM_NOT_FOUND).unwrap(), QuietDelete::Removed);
        for status in [
            INVALID_OWNER_EDIT,
            INTERACTION_NOT_ALLOWED,
            AUTH_FAILED,
            USER_CANCELED,
        ] {
            assert_eq!(outcome(status).unwrap(), QuietDelete::NeedsAuthorization);
        }
        // errSecInvalidKeychain:钥匙串本身出了问题,照常报错。
        let error = outcome(-25295).unwrap_err();
        assert!(matches!(&error, CoreError::Secret(message) if message.contains("-25295")));
    }

    /// 会在登录钥匙串里建、删条目(服务名只给这个测试用),默认不跑:
    /// `cargo test -p basalt-core secret -- --ignored`。
    #[test]
    #[ignore = "会写登录钥匙串"]
    fn deletes_own_items_and_leaves_foreign_ones_without_prompting() {
        const SERVICE: &str = "dev.basalt.keychaintest";
        let store = KeychainStore::new(SERVICE);
        store.set("own", "not-a-real-secret").unwrap();
        assert_eq!(store.delete_quietly("own").unwrap(), QuietDelete::Removed);
        assert_eq!(store.get("own").unwrap(), None);
        assert_eq!(store.delete_quietly("own").unwrap(), QuietDelete::Removed);

        // `security` 命令存的条目只归它自己,相当于旧版 Basalt 存下的密码。
        let added = Command::new("security")
            .args(["add-generic-password", "-s", SERVICE, "-a", "foreign"])
            .args(["-w", "not-a-real-secret"])
            .status()
            .unwrap();
        assert!(added.success());
        let outcome = store.delete_quietly("foreign");
        let cleaned = Command::new("security")
            .args(["delete-generic-password", "-s", SERVICE, "-a", "foreign"])
            .output()
            .unwrap();
        assert_eq!(outcome.unwrap(), QuietDelete::NeedsAuthorization);
        assert!(cleaned.status.success(), "不该删掉的条目应该还在");
    }
}
