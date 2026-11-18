use superagent_iam::{
    redact, scan_for_secrets, Action, Claims, IamError, IamServer, ResourceKind, ResourceRef,
};

const SECRET: &str = "super-secret-signing-key";
const NOW: i64 = 1_700_000_000;

fn claims(role: &str, tenant: &str) -> Claims {
    Claims {
        sub: "u1".into(),
        role: role.into(),
        tenant: tenant.into(),
        exp: NOW + 3600,
    }
}

fn res(kind: ResourceKind, tenant: &str) -> ResourceRef {
    ResourceRef {
        kind,
        tenant: tenant.into(),
        id: "r1".into(),
    }
}

fn grants(role: &str, kind: ResourceKind, action: Action) -> bool {
    match role {
        "admin" => true,
        "member" => match kind {
            ResourceKind::Tenant => action == Action::Read,
            _ => matches!(action, Action::Read | Action::Write),
        },
        "viewer" => action == Action::Read,
        _ => false,
    }
}

#[test]
fn jwt_auth_roundtrip_and_reject() {
    let mut s = IamServer::new(SECRET);
    s.add_user("u1", "member", "t1");
    let token = s.issue_token("u1", NOW, 3600).unwrap();
    let c = s.authenticate(&token, NOW + 10).unwrap();
    assert_eq!(c.sub, "u1");
    assert_eq!(c.role, "member");
    assert_eq!(c.tenant, "t1");
    // expired
    assert!(matches!(
        s.authenticate(&token, NOW + 3600),
        Err(IamError::Expired)
    ));
    // wrong secret
    let other = IamServer::new("different-secret");
    assert!(matches!(
        other.authenticate(&token, NOW),
        Err(IamError::InvalidSignature)
    ));
    // tampered payload (flip a char in the payload segment)
    let parts: Vec<&str> = token.split('.').collect();
    let tampered = format!("{}.{}X.{}", parts[0], parts[1], parts[2]);
    assert!(matches!(
        s.authenticate(&tampered, NOW),
        Err(IamError::InvalidSignature)
    ));
    // malformed
    assert!(matches!(
        s.authenticate("not-a-jwt", NOW),
        Err(IamError::MalformedToken)
    ));
    assert!(matches!(
        s.issue_token("ghost", NOW, 3600),
        Err(IamError::UnknownUser(_))
    ));
}

#[test]
fn authz_matrix_full_pass() {
    let mut s = IamServer::new(SECRET);
    let roles = ["admin", "member", "viewer"];
    let kinds = [
        ResourceKind::Tenant,
        ResourceKind::Thread,
        ResourceKind::Run,
        ResourceKind::Artifact,
    ];
    let actions = [Action::Read, Action::Write, Action::Delete, Action::Admin];
    let mut checked = 0;
    for role in roles {
        for kind in kinds {
            for action in actions {
                for same_tenant in [true, false] {
                    let rt = if same_tenant { "t1" } else { "t2" };
                    let expected = same_tenant && grants(role, kind, action);
                    let result = s.authorize(&claims(role, "t1"), &res(kind, rt), action);
                    assert_eq!(
                        result.is_ok(), expected,
                        "matrix mismatch role={role} kind={kind:?} action={action:?} same_tenant={same_tenant}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 96);
    assert_eq!(s.audit().len(), 96);
}

#[test]
fn resource_level_tenant_isolation() {
    let mut s = IamServer::new(SECRET);
    // admin in t1 still cannot touch t2 resources (hard tenant boundary).
    for kind in [
        ResourceKind::Tenant,
        ResourceKind::Thread,
        ResourceKind::Run,
        ResourceKind::Artifact,
    ] {
        assert!(matches!(
            s.authorize(&claims("admin", "t1"), &res(kind, "t2"), Action::Admin),
            Err(IamError::CrossTenant(_))
        ));
    }
    // same-tenant admin is allowed.
    assert!(s
        .authorize(
            &claims("admin", "t1"),
            &res(ResourceKind::Run, "t1"),
            Action::Admin
        )
        .is_ok());
}

#[test]
fn channel_identity_mapping() {
    let mut s = IamServer::new(SECRET);
    s.add_user("u1", "member", "t1");
    s.bind_channel("slack", "U123", "u1").unwrap();
    let u = s.map_channel_identity("slack", "U123").unwrap();
    let uid = u.user_id.clone();
    let role = u.role.clone();
    let tenant = u.tenant.clone();
    assert_eq!(uid, "u1");
    assert_eq!(role, "member");
    assert_eq!(tenant, "t1");
    assert!(matches!(
        s.map_channel_identity("slack", "nobody"),
        Err(IamError::UnknownIdentity(_))
    ));
    assert!(matches!(
        s.bind_channel("slack", "U999", "ghost"),
        Err(IamError::UnknownUser(_))
    ));
    // mapped identity can be issued a token that authenticates back to the user.
    let token = s.issue_token(&uid, NOW, 3600).unwrap();
    assert_eq!(s.authenticate(&token, NOW).unwrap().sub, "u1");
}

#[test]
fn credential_redaction_scan() {
    let mut s = IamServer::new(SECRET);
    s.add_user("u1", "admin", "t1");
    let token = s.issue_token("u1", NOW, 3600).unwrap();
    s.authorize(
        &claims("admin", "t1"),
        &res(ResourceKind::Run, "t1"),
        Action::Write,
    )
    .unwrap();
    // raw audit never carries the secret or the token.
    let raw = serde_json::to_string(s.audit()).unwrap();
    assert!(scan_for_secrets(&raw, &[SECRET, &token]).is_empty());
    // redacted diagnostic is clean even if secrets were present.
    let redacted = s.redacted_audit(&[SECRET, &token]);
    assert!(scan_for_secrets(&redacted, &[SECRET, &token]).is_empty());
    // redaction masks occurrences.
    assert!(
        redact(&format!("token={token} key={SECRET}"), &[SECRET, &token]).contains("[REDACTED]")
    );
    // positive control: the scanner DOES detect a genuine leak.
    assert_eq!(
        scan_for_secrets(&format!("leak {SECRET}"), &[SECRET]).len(),
        1
    );
}

#[test]
fn role_policy_and_unknown_role() {
    let mut s = IamServer::new(SECRET);
    // unknown role denied even within tenant.
    assert!(matches!(
        s.authorize(
            &claims("ghost", "t1"),
            &res(ResourceKind::Run, "t1"),
            Action::Read
        ),
        Err(IamError::Forbidden(_))
    ));
    // viewer cannot write; member cannot delete; admin can delete.
    assert!(matches!(
        s.authorize(
            &claims("viewer", "t1"),
            &res(ResourceKind::Thread, "t1"),
            Action::Write
        ),
        Err(IamError::Forbidden(_))
    ));
    assert!(matches!(
        s.authorize(
            &claims("member", "t1"),
            &res(ResourceKind::Artifact, "t1"),
            Action::Delete
        ),
        Err(IamError::Forbidden(_))
    ));
    assert!(s
        .authorize(
            &claims("admin", "t1"),
            &res(ResourceKind::Artifact, "t1"),
            Action::Delete
        )
        .is_ok());
}
