//! Runs the production TypeSafe client against a local, labeled text dataset.
//! Credential arrives on stdin and is never included in the output.
use riviu_core::typesafe::{Client, Support};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead};

#[derive(Deserialize)]
struct Case {
    id: String,
    candidate: String,
    caption: Option<String>,
    transcript: Option<String>,
    expected: Support,
}
#[derive(Serialize)]
struct Record {
    id: String,
    expected: Support,
    result: Option<riviu_core::typesafe::Verdict>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: typesafe_eval CASES.json"))?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(input)?)?;
    let mut key = String::new();
    io::stdin().lock().read_line(&mut key)?;
    let client = Client::new(key.trim().to_owned());
    for case in cases {
        let result = client
            .check(
                &case.candidate,
                case.caption.as_deref(),
                case.transcript.as_deref(),
            )
            .await;
        let record = match result {
            Ok(result) => Record {
                id: case.id,
                expected: case.expected,
                result: Some(result),
                error: None,
            },
            Err(error) => Record {
                id: case.id,
                expected: case.expected,
                result: None,
                error: Some(error.to_string()),
            },
        };
        println!("{}", serde_json::to_string(&record)?);
        if record.error.is_some() {
            anyhow::bail!("TypeSafe evaluation stopped on service error");
        }
    }
    Ok(())
}
