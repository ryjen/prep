//! Trusted-core verification primitives. Neither fetches nor extracts untrusted content.
use prep_manifest::{GitCommit, LockedSource};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{self, Read};

#[derive(Debug)]
pub enum SourceVerificationError {
    NonImmutable,
    WrongKind,
    IdentityMismatch,
    TooLarge,
    Io(io::Error),
}
impl fmt::Display for SourceVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonImmutable => f.write_str("source is not immutable"),
            Self::WrongKind => f.write_str("source kind differs from lock"),
            Self::IdentityMismatch => f.write_str("source integrity differs from lock"),
            Self::TooLarge => f.write_str("source exceeds verification limit"),
            Self::Io(error) => write!(f, "source read failed: {error}"),
        }
    }
}
impl std::error::Error for SourceVerificationError {}

/// Compare independently observed checkout HEAD with the locked commit.
/// This does not itself prove object integrity or filesystem containment.
pub fn verify_git_head(source: &LockedSource, observed: &GitCommit) -> Result<(), SourceVerificationError> {
    match source {
        LockedSource::Git { commit, .. } if commit == observed => Ok(()),
        LockedSource::Git { .. } => Err(SourceVerificationError::IdentityMismatch),
        LockedSource::Path { .. } => Err(SourceVerificationError::NonImmutable),
        LockedSource::Archive { .. } => Err(SourceVerificationError::WrongKind),
    }
}

/// Digest-verifies a bounded input before archive extraction. Extraction requires
/// a separate containment-safe implementation and must not trust this alone.
pub fn verify_archive_bytes(
    source: &LockedSource,
    input: &mut impl Read,
    maximum: u64,
) -> Result<u64, SourceVerificationError> {
    let expected = match source {
        LockedSource::Archive { sha256, .. } => sha256,
        LockedSource::Path { .. } => return Err(SourceVerificationError::NonImmutable),
        LockedSource::Git { .. } => return Err(SourceVerificationError::WrongKind),
    };
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 8192];
    loop {
        let n = input.read(&mut buffer).map_err(SourceVerificationError::Io)?;
        if n == 0 { break; }
        total = total.checked_add(n as u64).ok_or(SourceVerificationError::TooLarge)?;
        if total > maximum { return Err(SourceVerificationError::TooLarge); }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != expected.as_str() {
        return Err(SourceVerificationError::IdentityMismatch);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prep_manifest::{PackageVersion, PluginName, ProviderIdentity, Sha256Digest, SourceUrl};
    use std::io::Cursor;
    fn archive() -> LockedSource {
        LockedSource::Archive {
            url: SourceUrl::parse("https://example.test/archive.tar").unwrap(),
            sha256: Sha256Digest::parse("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap(),
            provider: ProviderIdentity {
                name: PluginName::parse("archive").unwrap(),
                version: PackageVersion::parse("1.0.0").unwrap(),
            },
        }
    }
    #[test]
    fn archive_verification_is_bounded_and_fail_closed() {
        let source = archive();
        assert_eq!(verify_archive_bytes(&source, &mut Cursor::new(b"abc"), 3).unwrap(), 3);
        assert!(matches!(verify_archive_bytes(&source, &mut Cursor::new(b"abd"), 3), Err(SourceVerificationError::IdentityMismatch)));
        assert!(matches!(verify_archive_bytes(&source, &mut Cursor::new(b"abc"), 2), Err(SourceVerificationError::TooLarge)));
    }
}
