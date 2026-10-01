// Exercise the same validator used before compiling the desktop application.
#[allow(dead_code)]
#[path = "../google_app_build.rs"]
mod build_config;

#[test]
fn build_and_runtime_accept_the_same_link_connection_configs() {
    use riviu_core::google_oauth::GoogleOAuthClientConfig;
    use serde_json::json;

    let minimal = json!({"clientId":"fixture.apps.googleusercontent.com"});
    let mut cases = vec![(minimal.clone(), true)];
    for key in ["clientSecret", "pickerApiKey", "projectNumber"] {
        for (value, accepted) in [
            (json!(null), true),
            (json!(""), false),
            (json!("  "), false),
            (json!(1), false),
            (json!(false), false),
            (json!([]), false),
            (json!({}), false),
            (json!("a\nb"), false),
            (json!("x".repeat(4097)), false),
        ] {
            let mut input = minimal.clone();
            input[key] = value;
            cases.push((input, accepted));
        }
    }
    for (key, value) in [
        ("clientSecret", "application-secret"),
        ("pickerApiKey", "legacy-picker-key"),
        ("projectNumber", "12345"),
    ] {
        let mut input = minimal.clone();
        input[key] = json!(value);
        cases.push((input, true));
    }
    for key in ["accessToken", "refreshToken", "account", "unknown"] {
        let mut input = minimal.clone();
        input[key] = json!("do-not-echo-this-value");
        cases.push((input, false));
    }
    for client in [
        "",
        " ",
        "invalid",
        ".apps.googleusercontent.com",
        "é.apps.googleusercontent.com",
    ] {
        cases.push((json!({"clientId":client}), false));
    }
    cases.push((json!({"clientId":12}), false));
    cases.push((
        json!({"clientId":"x".repeat(300)+".apps.googleusercontent.com"}),
        false,
    ));
    cases.push((
        json!({"clientId":"fixture.apps.googleusercontent.com","projectNumber":"1".repeat(33)}),
        false,
    ));
    for (input, accepted) in cases {
        let runtime = serde_json::from_value::<GoogleOAuthClientConfig>(input.clone())
            .is_ok_and(|config| config.validate().is_ok());
        let build = build_config::validate(&input.to_string(), true);
        assert_eq!(
            runtime, accepted,
            "runtime rejected or admitted wrong shape"
        );
        assert_eq!(build.is_ok(), accepted, "build/runtime contract drift");
        if let Err(error) = build {
            assert!(!error.contains("do-not-echo-this-value"));
        }
    }
    // The serialized build input has its own bounded-file policy.
    assert!(build_config::validate(&(minimal.to_string() + &" ".repeat(16 * 1024)), true).is_err());
    assert!(build_config::validate("", true).is_err());
    assert!(build_config::validate("", false).is_ok());
}
