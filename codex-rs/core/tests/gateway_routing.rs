use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ModelRerouteReason;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_response_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::sse_response;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_routing_is_informational_and_preserves_next_request_model() -> Result<()> {
    let server = start_mock_server().await;
    let group = "group/workbuddy-failover";
    let responses = [false, true]
        .into_iter()
        .enumerate()
        .map(|(i, fallback)| {
            let id = format!("resp-{i}");
            sse_response(sse(vec![
                json!({"type":"response.magpie_routing","routing":{
                    "from_model": if fallback { "workbuddy/astra" } else { group },
                    "to_model": "workbuddy/sol", "fallback":fallback
                }}),
                ev_response_created(&id),
                ev_assistant_message(&format!("msg-{i}"), "OK"),
                ev_completed(&id),
            ]))
        })
        .collect();
    let mock = mount_response_sequence(&server, responses).await;
    let mut builder = test_codex().with_model(group);
    let test = builder.build_with_auto_env(&server).await?;
    for reason in [
        ModelRerouteReason::GatewaySelected,
        ModelRerouteReason::GatewayFailover,
    ] {
        test.codex
            .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
                text: "Reply OK".to_string(),
                text_elements: vec![],
            }]))
            .await?;
        let mut rerouted = false;
        loop {
            match wait_for_event(&test.codex, |_| true).await {
                EventMsg::ModelReroute(event) => {
                    assert_eq!(event.reason, reason);
                    assert_eq!(event.to_model, "workbuddy/sol");
                    rerouted = true;
                }
                EventMsg::Warning(warning) => assert!(
                    !warning.message.contains("high-risk cyber"),
                    "{}",
                    warning.message
                ),
                EventMsg::Error(error) => panic!("unexpected error: {}", error.message),
                EventMsg::TurnComplete(_) => break,
                _ => {}
            }
        }
        assert!(rerouted);
    }
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.body_json()["model"], group);
        assert_eq!(
            request.header("x-magpie-routing-notice").as_deref(),
            Some("v1")
        );
    }
    Ok(())
}
