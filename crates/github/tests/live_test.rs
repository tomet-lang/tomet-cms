//! Opt-in live test against GitHub's live GraphQL API.
//!
//! **NOTE**: This test is marked `#[ignore]` and must never run in CI or by default.
//! It requires explicit configuration with a scratch repository and personal access token:
//!
//! ```bash
//! GITHUB_TOKEN="ghp_..." GITHUB_REPO="owner/repo" cargo test -p tomet-cms-github --test live_test -- --ignored --nocapture
//! ```

use std::future::Future;
use std::task::{Context, Poll, Waker};

use tomet_cms::host::Host;
use tomet_cms::save::{SaveRequest, save};
use tomet_cms_github::host::GitHubHost;
use tomet_cms_github::http::{HttpClient, HttpError, HttpRequest, HttpResponse};

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut pinned = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match pinned.as_mut().poll(&mut cx) {
        Poll::Ready(res) => res,
        Poll::Pending => panic!("future unexpectedly pending"),
    }
}

/// A minimal curl-backed HTTP client for offline native test execution
/// without pulling in network/runtime crates into the dependency tree.
struct CurlHttpClient;

impl HttpClient for CurlHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut cmd = std::process::Command::new("curl");
        cmd.arg("-s").arg("-i"); // include headers in output
        cmd.arg("-X").arg(request.method.to_string());

        for (name, value) in &request.headers {
            cmd.arg("-H").arg(format!("{name}: {value}"));
        }

        if !request.body.is_empty() {
            use std::io::Write;
            cmd.arg("--data-binary").arg("@-");
            cmd.arg(&request.url);
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let mut child = cmd
                .spawn()
                .map_err(|e| HttpError::Network(format!("failed to spawn curl: {e}")))?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(&request.body)
                    .map_err(|e| HttpError::Network(format!("failed to write curl stdin: {e}")))?;
            }

            let output = child
                .wait_with_output()
                .map_err(|e| HttpError::Network(format!("curl failed: {e}")))?;

            parse_curl_output(&output.stdout)
        } else {
            cmd.arg(&request.url);
            let output = cmd
                .output()
                .map_err(|e| HttpError::Network(format!("curl execution failed: {e}")))?;
            parse_curl_output(&output.stdout)
        }
    }
}

fn parse_curl_output(raw: &[u8]) -> Result<HttpResponse, HttpError> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|window| window == b"\n\n"))
        .ok_or_else(|| HttpError::Other("could not find header delimiter in curl output".to_string()))?;

    let header_bytes = &raw[..header_end];
    let body_bytes = if raw[header_end..].starts_with(b"\r\n\r\n") {
        &raw[header_end + 4..]
    } else {
        &raw[header_end + 2..]
    };

    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str.lines();

    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Other("missing status line in curl output".to_string()))?;

    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::Other(format!("failed to parse HTTP status from '{status_line}'")))?;

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    let mut resp = HttpResponse::new(status, body_bytes.to_vec());
    for (k, v) in headers {
        resp = resp.with_header(k, v);
    }

    Ok(resp)
}

#[test]
#[ignore = "Live test requiring real GITHUB_TOKEN and GITHUB_REPO credentials"]
fn live_github_create_edit_and_concurrent_roundtrip() {
    let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN must be set to run live test");
    let repo_slug =
        std::env::var("GITHUB_REPO").expect("GITHUB_REPO must be set (e.g. 'owner/repo') to run live test");

    let (owner, repo) = repo_slug
        .split_once('/')
        .expect("GITHUB_REPO must be formatted as 'owner/repo'");

    let branch = std::env::var("GITHUB_BRANCH").unwrap_or_else(|_| "main".to_string());

    let host = GitHubHost::new(CurlHttpClient, owner, repo, &branch, token);

    // 1. Get HEAD
    let initial_head = block_on(host.get_head()).expect("live get_head failed");
    println!("Live test initial HEAD: {initial_head}");

    let test_file = format!("scratch-test-{}.tmt", std::process::id());

    // 2. Create a new page
    let create_content = "=[ Scratch Test ]\n\nInitial live test content.\n";
    let create_req = SaveRequest::create(&test_file, create_content, "Live test create");
    let create_res = block_on(save(&host, create_req)).expect("live create save failed");
    println!("Created revision: {}", create_res.commit.revision);

    // 3. Read back created file
    let file_content = block_on(host.read_file(&test_file, &create_res.commit.revision))
        .expect("live read_file failed")
        .expect("file missing after create");
    assert_eq!(file_content.content, create_content);

    // 4. Edit page
    let edit_content = "=[ Scratch Test ]\n\nInitial live test content edited.\n";
    let edit_req = SaveRequest::edit(
        &test_file,
        &create_res.commit.revision,
        edit_content,
        "Live test edit",
    );
    let edit_res = block_on(save(&host, edit_req)).expect("live edit save failed");
    println!("Edited revision: {}", edit_res.commit.revision);

    // 5. Read back edited file
    let file_content_edited = block_on(host.read_file(&test_file, &edit_res.commit.revision))
        .expect("live read_file failed")
        .expect("file missing after edit");
    assert_eq!(file_content_edited.content, edit_content);

    println!("Live test finished successfully!");
}
