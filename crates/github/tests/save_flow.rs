//! End-to-end integration tests driving `tomet_cms::save` through `GitHubHost`
//! with `FakeHttpClient`.

use std::future::Future;
use std::task::{Context, Poll, Waker};

use tomet_cms::host::Revision;
use tomet_cms::save::{SaveRequest, save};
use tomet_cms_github::host::GitHubHost;
use tomet_cms_github::http::{FakeHttpClient, HttpResponse};

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut pinned = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match pinned.as_mut().poll(&mut cx) {
        Poll::Ready(res) => res,
        Poll::Pending => panic!("future unexpectedly pending"),
    }
}

fn json_head_response(oid: &str) -> HttpResponse {
    let body = serde_json::to_vec(&serde_json::json!({
        "data": {
            "repository": {
                "ref": {
                    "target": {
                        "oid": oid
                    }
                }
            }
        }
    }))
    .unwrap();
    HttpResponse::new(200, body).with_header("content-type", "application/json")
}

fn json_blob_response(text: Option<&str>) -> HttpResponse {
    let body = serde_json::to_vec(&serde_json::json!({
        "data": {
            "repository": {
                "object": text.map(|t| {
                    serde_json::json!({
                        "isBinary": false,
                        "isTruncated": false,
                        "text": t,
                        "byteSize": t.len()
                    })
                })
            }
        }
    }))
    .unwrap();
    HttpResponse::new(200, body).with_header("content-type", "application/json")
}

fn json_commit_response(new_oid: &str) -> HttpResponse {
    let body = serde_json::to_vec(&serde_json::json!({
        "data": {
            "createCommitOnBranch": {
                "commit": {
                    "oid": new_oid
                }
            }
        }
    }))
    .unwrap();
    HttpResponse::new(200, body).with_header("content-type", "application/json")
}

#[test]
fn end_to_end_clean_create_flow() {
    let client = FakeHttpClient::new();

    // 1. get_head query returns current HEAD
    client.queue_response(json_head_response("1111111111111111111111111111111111111111"));
    // 2. read_file at HEAD returns absent (null object)
    client.queue_response(json_blob_response(None));
    // 3. commit_file mutation succeeds
    client.queue_response(json_commit_response("2222222222222222222222222222222222222222"));

    let host = GitHubHost::new(client.clone(), "my-org", "my-wiki", "main", "token123");
    let content = "=[ Welcome ]\n\nInitial wiki page.\n";
    let req = SaveRequest::create("welcome.tmt", content, "Create welcome page");

    let res = block_on(save(&host, req)).unwrap();

    assert_eq!(
        res.commit.revision.as_str(),
        "2222222222222222222222222222222222222222"
    );
    assert!(!res.has_conflicts);
    assert!(res.warnings.is_empty());
    assert_eq!(client.remaining_responses(), 0);
}

#[test]
fn end_to_end_save_retries_once_after_branch_moved() {
    let client = FakeHttpClient::new();
    let base_rev = Revision::new("0000000000000000000000000000000000000000");

    let initial_text = "=[ Title ]\n\nInitial text.\n";
    let edited_text = "=[ Title ]\n\nEdited text locally.\n";

    // Attempt 1:
    // 1. get_head returns head_1
    client.queue_response(json_head_response("1111111111111111111111111111111111111111"));
    // 2. read_file at head_1 (remote content equals base)
    client.queue_response(json_blob_response(Some(initial_text)));
    // 3. read_file at base_rev
    client.queue_response(json_blob_response(Some(initial_text)));
    // 4. commit_file fails (another commit landed concurrently)
    client.queue_response(
        HttpResponse::new(
            200,
            serde_json::to_vec(&serde_json::json!({
                "data": null,
                "errors": [{"message": "Head branch was modified"}]
            }))
            .unwrap(),
        )
        .with_header("content-type", "application/json"),
    );
    // 5. follow-up get_head triggered on commit failure returns head_2
    client.queue_response(json_head_response("3333333333333333333333333333333333333333"));

    // Attempt 2 (retry):
    // 6. get_head returns head_2
    client.queue_response(json_head_response("3333333333333333333333333333333333333333"));
    // 7. read_file at head_2 (still equals base)
    client.queue_response(json_blob_response(Some(initial_text)));
    // 8. read_file at base_rev
    client.queue_response(json_blob_response(Some(initial_text)));
    // 9. commit_file succeeds with commit oid
    client.queue_response(json_commit_response("4444444444444444444444444444444444444444"));

    let host = GitHubHost::new(client.clone(), "my-org", "my-wiki", "main", "token123");
    let req = SaveRequest::edit("page.tmt", &base_rev, edited_text, "Update page");

    let res = block_on(save(&host, req)).unwrap();

    assert_eq!(
        res.commit.revision.as_str(),
        "4444444444444444444444444444444444444444"
    );
    assert!(!res.has_conflicts);
    assert!(res.warnings.is_empty());
    assert_eq!(client.remaining_responses(), 0);
}
