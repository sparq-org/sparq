// [GPT-6] Generated-calendar policy transitions verified over authenticated HTTP.
use super::{
    auth::Credentials,
    load::{send, Outgoing},
    Result, Settings,
};
use serde_json::{json, Value};
use sparq_acbench::population::{self, PopulationConfig, Service};
use std::time::Instant;

pub(super) async fn run(settings: Settings) -> Result<()> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        settings
            .path("corpus", "population-corpus")
            .join("manifest.json"),
    )?)?;
    let config: PopulationConfig = serde_json::from_value(manifest["config"].clone())?;
    let pod = settings.number("pod", 0)?;
    let state = settings.text("state", "revoke");
    if !["grant", "revoke", "probe-granted", "probe-revoked"].contains(&state.as_str()) {
        return Err("churn state must be grant, revoke, probe-granted or probe-revoked".into());
    }
    if !config.sharing_enabled {
        return Err("churn requires the generated sharing scenario".into());
    }
    let granted = state == "grant" || state == "probe-granted";
    let root = population::pod_root(&config, pod);
    let credentials = Credentials::load(&settings)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            settings.number("timeout-ms", 30000)?,
        ))
        .build()?;
    let connect = settings.text("connect", "http://127.0.0.1:3100");
    let calendar_query="SELECT (COUNT(?s) AS ?count) WHERE { GRAPH ?g { ?s <https://sparq.dev/bench/personal#service> <https://sparq.dev/bench/personal#calendar> } }";
    let owner = population::owner_webid(&config, pod);
    let recipient = population::recipient_webid(&config, pod, 0);
    if state == "grant" || state == "revoke" {
        let id = settings.text("mutation-id", &format!("churn-{state}"));
        let (graph, subject, predicate, object) = if manifest["model"] == "wac" {
            (
                format!("{root}calendar/.acl"),
                format!("{root}calendar/.acl#reader"),
                "http://www.w3.org/ns/auth/acl#agent",
                recipient.clone(),
            )
        } else {
            (
                format!("{root}calendar/.acr"),
                format!("{root}calendar/.acr#reader-control"),
                "http://www.w3.org/ns/solid/acp#apply",
                format!("{root}calendar/.acr#reader-policy"),
            )
        };
        let action = if granted { "INSERT" } else { "DELETE" };
        let query = format!(
            "{action} DATA {{ GRAPH <{graph}> {{ <{subject}> <{predicate}> <{object}> }} }}"
        );
        // Wrong owner, recipient and anonymous identities cannot use shared owner management.
        for (index, webid) in [
            Some(population::owner_webid(&config, pod + 1)),
            Some(recipient.clone()),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let record=send(&client,&credentials,&connect,Outgoing{path:format!("/pods/{pod}/policy"),query_text:query.clone(),webid,update:true,observation:None,scheduled:Instant::now(),record:json!({"record_type":"negative-policy-probe","pod":pod,"principal_case":index})}).await?;
            println!("{}", record);
            if record["status"] != 403 {
                return Err("unauthorized generated policy update did not fail closed".into());
            }
        }
        let record=send(&client,&credentials,&connect,Outgoing{path:format!("/pods/{pod}/policy"),query_text:query,webid:Some(owner.clone()),update:true,observation:Some(json!({"id":id,"records":[]})),scheduled:Instant::now(),record:json!({"record_type":"request","pod":pod,"mutation_id":id,"operation":"policy-attempt","state":state})}).await?;
        println!("{}", record);
        if record["outcome"] != "ok" || !record["mutation_receipt"].is_object() {
            return Err("policy change failed or missing durable receipt".into());
        }
        if settings.0.contains_key("expected-delta") {
            let expected: i64 = settings.text("expected-delta", "0").parse()?;
            if record["policy_triple_delta"] != expected {
                return Err("policy transition was not the expected effective change".into());
            }
        }
    }
    if settings.0.contains_key("evict-pod") {
        let other = settings.number("evict-pod", pod)?;
        if other == pod {
            return Err("eviction probe requires another Pod".into());
        }
        let record = send(
            &client,
            &credentials,
            &connect,
            Outgoing {
                path: format!("/pods/{other}/sparql"),
                query_text: calendar_query.into(),
                webid: Some(population::owner_webid(&config, other)),
                update: false,
                observation: None,
                scheduled: Instant::now(),
                record: json!({"record_type":"eviction-probe","pod":other}),
            },
        )
        .await?;
        println!("{}", record);
        if record["outcome"] != "ok" {
            return Err("eviction probe request failed".into());
        }
    }
    let total = population::planned_record_counts(&config, pod)?[&Service::Calendar];
    let months = u64::from(config.history_months);
    let shared = (0..config.history_months)
        .filter(|month| {
            population::can_read(&config, pod, Service::Calendar, *month, Some(&recipient))
        })
        .map(|month| total / months + u64::from(u64::from(month) < total % months))
        .sum::<u64>();
    for (label, webid, expected) in [
        ("owner", Some(owner), total),
        (
            "recipient",
            Some(recipient),
            if granted { shared } else { 0 },
        ),
        ("anonymous", None, 0),
    ] {
        let record=send(&client,&credentials,&connect,Outgoing{path:format!("/pods/{pod}/sparql"),query_text:calendar_query.into(),webid,update:false,observation:None,scheduled:Instant::now(),record:json!({"record_type":"authorization-probe","pod":pod,"principal":label,"expected_calendar_records":expected,"state":state})}).await?;
        println!("{}", record);
        if record["outcome"] != "ok"
            || record["count"].as_str().and_then(|v| v.parse::<u64>().ok()) != Some(expected)
        {
            return Err(format!(
                "generated calendar post-acknowledgement oracle mismatch for {label}"
            )
            .into());
        }
    }
    println!(
        "{}",
        json!({"record_type":"policy-churn-check-complete","pod":pod,"state":state,"inherited_and_private_exception_oracle":true})
    );
    Ok(())
}
