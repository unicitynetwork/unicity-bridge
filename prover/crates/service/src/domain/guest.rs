use crate::store::parse_hex32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VaultGuestKey([u8; 32]);

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum GuestKeyError {
    #[error("deployment record: {0}")]
    Record(String),
    #[error(
        "the guest ELF has verifying key {elf}, the vault accepts only {vault}: \
         every proof would be rejected on chain"
    )]
    Mismatch { elf: String, vault: String },
}

impl VaultGuestKey {
    pub fn from_deployment(json: &str) -> Result<Option<Self>, GuestKeyError> {
        let record: serde_json::Value =
            serde_json::from_str(json).map_err(|err| GuestKeyError::Record(err.to_string()))?;
        let Some(vkey) = record.pointer("/deployment/vkey") else {
            return Ok(None);
        };
        vkey.as_str()
            .and_then(parse_hex32)
            .map(|key| Some(Self(key)))
            .ok_or_else(|| {
                GuestKeyError::Record(format!("deployment.vkey is not 32 bytes of hex: {vkey}"))
            })
    }

    pub fn admit(&self, program_vkey: &str) -> Result<(), GuestKeyError> {
        if parse_hex32(program_vkey) == Some(self.0) {
            return Ok(());
        }
        Err(GuestKeyError::Mismatch {
            elf: program_vkey.to_string(),
            vault: self.to_string(),
        })
    }
}

impl std::fmt::Display for VaultGuestKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAULT: &str = "0x0039a5424014e57caf45d3451053e6c014547837ae09c9eb724aa569389b90d5";
    const OTHER: &str = "0x0036d4a9000000000000000000000000000000000000000000000000000000aa";

    fn record(vkey: &str) -> String {
        format!(r#"{{"config":{{}},"deployment":{{"vault":"0x9C2B","vkey":"{vkey}"}}}}"#)
    }

    #[test]
    fn reads_the_key_the_vault_was_deployed_with() {
        let key = VaultGuestKey::from_deployment(&record(VAULT))
            .unwrap()
            .unwrap();

        assert_eq!(key.to_string(), VAULT);
    }

    #[test]
    fn a_record_that_names_no_key_pins_nothing() {
        assert_eq!(VaultGuestKey::from_deployment(r#"{"config":{}}"#), Ok(None));
        assert_eq!(
            VaultGuestKey::from_deployment(r#"{"deployment":{"vault":"0x9C2B"}}"#),
            Ok(None)
        );
    }

    #[test]
    fn a_malformed_key_or_record_is_an_error_rather_than_no_pin() {
        assert!(matches!(
            VaultGuestKey::from_deployment(&record("0x0039")),
            Err(GuestKeyError::Record(_))
        ));
        assert!(matches!(
            VaultGuestKey::from_deployment("not json"),
            Err(GuestKeyError::Record(_))
        ));
    }

    #[test]
    fn admits_the_program_whose_key_the_vault_holds_in_either_spelling() {
        let key = VaultGuestKey::from_deployment(&record(VAULT))
            .unwrap()
            .unwrap();

        assert_eq!(key.admit(VAULT), Ok(()));
        assert_eq!(key.admit(&VAULT[2..].to_uppercase()), Ok(()));
    }

    #[test]
    fn refuses_any_other_program_and_names_both_keys() {
        let key = VaultGuestKey::from_deployment(&record(VAULT))
            .unwrap()
            .unwrap();

        assert_eq!(
            key.admit(OTHER),
            Err(GuestKeyError::Mismatch {
                elf: OTHER.to_string(),
                vault: VAULT.to_string()
            })
        );
        assert!(matches!(
            key.admit("garbage"),
            Err(GuestKeyError::Mismatch { .. })
        ));
    }
}
