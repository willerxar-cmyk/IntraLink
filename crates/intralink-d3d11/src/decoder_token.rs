//! Create and inspect a reduced primary token for only the decoder child.
//! Low integrity + write-restricted SID + privilege removal is defense in
//! depth, not a filesystem-read/network/AppContainer security sandbox.
use std::{
    mem::{align_of, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};

use anyhow::{Context, Result, ensure};
use windows::{
    Win32::{
        Foundation::{HANDLE, LUID},
        Security::*,
        System::{
            SystemServices::{SE_GROUP_INTEGRITY, SE_GROUP_USE_FOR_DENY_ONLY},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::PCWSTR,
};

pub(super) fn handle(owner: &OwnedHandle) -> HANDLE {
    HANDLE(owner.as_raw_handle())
}

/// Take ownership of a successful native handle-returning call.
pub(super) fn owned(raw: HANDLE) -> Result<OwnedHandle> {
    ensure!(!raw.is_invalid(), "Invalid native owned handle");
    // SAFETY: callers pass a newly acquired, non-pseudo handle exactly once.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw.0) })
}

fn current_token(access: TOKEN_ACCESS_MASK) -> Result<OwnedHandle> {
    let mut raw = HANDLE::default();
    // SAFETY: current-process pseudo handle and initialized out parameter.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), access, &mut raw)?;
    }
    owned(raw)
}

/// Aligned buffer keeps the native token information and embedded SIDs alive.
struct Info {
    storage: Vec<usize>,
    len: usize,
}

impl Info {
    fn query(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Self> {
        let mut size = 0u32;
        // SAFETY: size-query convention; no output buffer on the first call.
        unsafe {
            let _ = GetTokenInformation(token, class, None, 0, &mut size);
        }
        ensure!(
            (1..=256 * 1024).contains(&size),
            "Invalid token information size"
        );
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        let capacity = storage.len() * size_of::<usize>();
        // SAFETY: aligned initialized storage of the exact advertised size.
        unsafe {
            GetTokenInformation(
                token,
                class,
                Some(storage.as_mut_ptr().cast()),
                capacity as u32,
                &mut size,
            )?;
        }
        ensure!(
            size as usize <= capacity,
            "Token information length overflow"
        );
        Ok(Self {
            storage,
            len: size as usize,
        })
    }

    fn get<T: Copy>(&self) -> Result<T> {
        ensure!(
            self.len >= size_of::<T>() && align_of::<T>() <= align_of::<usize>(),
            "Truncated token information"
        );
        // SAFETY: OS returned the caller-selected native structure; size and
        // alignment were checked. Callers use only POD Windows token structs.
        Ok(unsafe { self.storage.as_ptr().cast::<T>().read() })
    }
}

struct Sid([u32; 17]);
impl Sid {
    fn new(kind: WELL_KNOWN_SID_TYPE) -> Result<Self> {
        let mut sid = Self([0; 17]);
        let mut len = SECURITY_MAX_SID_SIZE;
        // SAFETY: aligned 68-byte SID storage; no domain SID is required.
        unsafe {
            CreateWellKnownSid(kind, None, Some(sid.as_psid()), &mut len)?;
        }
        Ok(sid)
    }
    fn as_psid(&mut self) -> PSID {
        PSID(self.0.as_mut_ptr().cast())
    }
}

pub(super) fn create() -> Result<OwnedHandle> {
    let original =
        current_token(TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT)?;
    let user_info = Info::query(handle(&original), TokenUser)?;
    let user = user_info.get::<TOKEN_USER>()?;
    // Retain the existing interactive logon SID on the restricted access
    // check. The native window station/desktop may grant this SID, rather
    // than the account SID. No new principal or desktop ACL is introduced.
    let logon_info = Info::query(handle(&original), TokenLogonSid)?;
    let logon = logon_info.get::<TOKEN_GROUPS>()?;
    ensure!(
        logon.GroupCount == 1,
        "Expected exactly one decoder logon SID"
    );
    let mut admin = Sid::new(WinBuiltinAdministratorsSid)?;
    let mut power = Sid::new(WinBuiltinPowerUsersSid)?;
    let mut restricted = Sid::new(WinRestrictedCodeSid)?;
    let disabled = [
        SID_AND_ATTRIBUTES {
            Sid: admin.as_psid(),
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: power.as_psid(),
            Attributes: 0,
        },
    ];
    // The user SID is required on the second access check for writes to
    // user-owned resources; MIC still blocks writes to higher-label objects.
    // The OS/driver may deny access under this policy; never loosen silently.
    let restricting = [
        SID_AND_ATTRIBUTES {
            Sid: user.User.Sid,
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: restricted.as_psid(),
            Attributes: 0,
        },
        SID_AND_ATTRIBUTES {
            Sid: logon.Groups[0].Sid,
            Attributes: 0,
        },
    ];
    let mut raw = HANDLE::default();
    // SAFETY: all SID pointers remain in their owning buffers through the call.
    unsafe {
        CreateRestrictedToken(
            handle(&original),
            DISABLE_MAX_PRIVILEGE | WRITE_RESTRICTED,
            Some(&disabled),
            None,
            Some(&restricting),
            &mut raw,
        )
    }
    .context("Create reduced decoder primary token")?;
    let token = owned(raw)?;
    if integrity_rid(handle(&token))? > 0x1000 {
        let mut low = Sid::new(WinLowLabelSid)?;
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low.as_psid(),
                Attributes: SE_GROUP_INTEGRITY as u32,
            },
        };
        // SAFETY: label and referenced valid SID are live for the call.
        unsafe {
            SetTokenInformation(
                handle(&token),
                TokenIntegrityLevel,
                (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(label.Label.Sid),
            )
        }
        .context("Set low integrity on decoder token")?;
    }
    verify_token(handle(&token))?;
    Ok(token)
}

/// Verify the actual primary token of the current decoder process. No
/// environment variable or bootstrap bit can stand in for the OS token.
pub(crate) fn verify_current() -> Result<()> {
    let token = current_token(TOKEN_QUERY)?;
    verify_token(handle(&token))
}

fn verify_token(token: HANDLE) -> Result<()> {
    // SAFETY: token is live and queryable. IsTokenRestricted is false when
    // the token has no restricting SIDs.
    unsafe { IsTokenRestricted(token) }.context("Decoder token is not restricted")?;
    ensure!(
        integrity_rid(token)? <= 0x1000,
        "Decoder integrity level exceeds low"
    );
    let mandatory = Info::query(token, TokenMandatoryPolicy)?.get::<TOKEN_MANDATORY_POLICY>()?;
    ensure!(
        mandatory.Policy.0 & TOKEN_MANDATORY_POLICY_NO_WRITE_UP.0 != 0,
        "Decoder mandatory no-write-up policy unavailable"
    );
    let restrictions = Info::query(token, TokenRestrictedSids)?;
    let user_info = Info::query(token, TokenUser)?;
    let user = user_info.get::<TOKEN_USER>()?;
    let logon_info = Info::query(token, TokenLogonSid)?;
    let logon = logon_info.get::<TOKEN_GROUPS>()?;
    ensure!(
        logon.GroupCount == 1,
        "Unexpected decoder logon group count"
    );
    verify_groups(&restrictions, Some((user.User.Sid, logon.Groups[0].Sid)))?;
    let groups = Info::query(token, TokenGroups)?;
    verify_groups(&groups, None)?;

    let privileges = Info::query(token, TokenPrivileges)?;
    let count = privileges.get::<u32>()? as usize;
    let start = std::mem::offset_of!(TOKEN_PRIVILEGES, Privileges);
    ensure!(
        count <= 1 && start + count * size_of::<LUID_AND_ATTRIBUTES>() <= privileges.len,
        "Decoder retains excess privileges"
    );
    let mut notify = LUID::default();
    // SAFETY: lookup the fixed OS privilege name into local initialized storage.
    unsafe {
        LookupPrivilegeValueW(PCWSTR::null(), SE_CHANGE_NOTIFY_NAME, &mut notify)?;
    }
    if count == 1 {
        // SAFETY: the one LUID entry was size-checked and the native buffer is aligned.
        let actual = unsafe {
            privileges
                .storage
                .as_ptr()
                .cast::<u8>()
                .add(start)
                .cast::<LUID_AND_ATTRIBUTES>()
                .read()
        };
        ensure!(actual.Luid == notify, "Unexpected decoder privilege");
    }
    Ok(())
}

fn verify_groups(info: &Info, restricting: Option<(PSID, PSID)>) -> Result<()> {
    let count = info.get::<u32>()? as usize;
    let offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    ensure!(
        count <= 2048 && offset + count * size_of::<SID_AND_ATTRIBUTES>() <= info.len,
        "Invalid token group array"
    );
    let mut admin = Sid::new(WinBuiltinAdministratorsSid)?;
    let mut power = Sid::new(WinBuiltinPowerUsersSid)?;
    let mut restricted = Sid::new(WinRestrictedCodeSid)?;
    let admin = admin.as_psid();
    let power = power.as_psid();
    let restricted = restricted.as_psid();
    let mut matches = [0usize; 3];
    if restricting.is_some() {
        ensure!(count == 3, "Unexpected decoder restricting SID count");
    }
    for index in 0..count {
        // SAFETY: each native SID_AND_ATTRIBUTES record is within the checked
        // OS token buffer; all embedded SIDs remain live for comparisons.
        let group = unsafe {
            info.storage
                .as_ptr()
                .cast::<u8>()
                .add(offset + index * size_of::<SID_AND_ATTRIBUTES>())
                .cast::<SID_AND_ATTRIBUTES>()
                .read()
        };
        // SAFETY: compare OS-returned group SID with valid well-known SIDs.
        let (is_admin, is_power, is_restricted) = unsafe {
            (
                EqualSid(group.Sid, admin).is_ok(),
                EqualSid(group.Sid, power).is_ok(),
                EqualSid(group.Sid, restricted).is_ok(),
            )
        };
        if let Some((user, logon)) = restricting {
            // SAFETY: both SIDs are valid and their OS-returned buffers
            // remain live in verify_token for this entire comparison.
            let (is_user, is_logon) = unsafe {
                (
                    EqualSid(group.Sid, user).is_ok(),
                    EqualSid(group.Sid, logon).is_ok(),
                )
            };
            ensure!(
                is_user || is_logon || is_restricted,
                "Unexpected restricting SID"
            );
            matches[0] += usize::from(is_user);
            matches[1] += usize::from(is_logon);
            matches[2] += usize::from(is_restricted);
        } else if is_admin || is_power {
            ensure!(
                group.Attributes & SE_GROUP_USE_FOR_DENY_ONLY as u32 != 0,
                "Elevated decoder group remains enabled"
            );
        }
    }
    ensure!(
        restricting.is_none() || matches == [1, 1, 1],
        "Missing or duplicate restricting SID"
    );
    Ok(())
}

fn integrity_rid(token: HANDLE) -> Result<u32> {
    let integrity = Info::query(token, TokenIntegrityLevel)?;
    let label = integrity.get::<TOKEN_MANDATORY_LABEL>()?;
    // SAFETY: Windows returned a valid mandatory-label SID in the live
    // information buffer; the last sub-authority is its integrity RID.
    let rid = unsafe {
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        ensure!(count > 0, "Invalid integrity SID");
        *GetSidSubAuthority(label.Label.Sid, u32::from(count - 1))
    };
    Ok(rid)
}

/// Real token-only diagnostic. No worker process, GPU, vault or network is
/// created. The caller's original primary token must remain unchanged.
pub fn verify_decoder_token_creation() -> Result<()> {
    let parent = current_token(TOKEN_QUERY)?;
    let before = Info::query(handle(&parent), TokenStatistics)?.get::<TOKEN_STATISTICS>()?;
    let parent_rid = integrity_rid(handle(&parent))?;
    let reduced = create()?;
    let child = Info::query(handle(&reduced), TokenStatistics)?.get::<TOKEN_STATISTICS>()?;
    let child_rid = integrity_rid(handle(&reduced))?;
    let after = Info::query(handle(&parent), TokenStatistics)?.get::<TOKEN_STATISTICS>()?;
    ensure!(
        before.TokenId == after.TokenId
            && before.ModifiedId == after.ModifiedId
            && before.PrivilegeCount == after.PrivilegeCount
            && parent_rid == integrity_rid(handle(&parent))?,
        "Parent token changed during decoder token creation"
    );
    ensure!(
        child.TokenId != before.TokenId
            && child.AuthenticationId == before.AuthenticationId
            && child.TokenType == TokenPrimary
            && child.PrivilegeCount <= 1
            && child_rid <= parent_rid,
        "Unexpected reduced primary token"
    );
    println!(
        "decoder_token_check=passed, parent_integrity=0x{parent_rid:x}, child_integrity=0x{child_rid:x}, parent_privileges={}, child_privileges={}, restricting_sids=3, no_write_up=true, parent_unchanged=true",
        before.PrivilegeCount, child.PrivilegeCount
    );
    Ok(())
}
