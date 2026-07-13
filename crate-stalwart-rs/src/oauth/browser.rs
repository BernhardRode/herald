//! Authorization Code + PKCE browser flow with loopback callback listener.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;

use tracing::{info, warn};

use super::{OAuthError, OAuthTokenResponse, StalwartOAuth};

impl StalwartOAuth {
    /// Perform the full OAuth2 Authorization Code flow with PKCE.
    ///
    /// 1. Discovers OAuth metadata
    /// 2. Opens browser to authorization endpoint
    /// 3. Listens on localhost for the redirect callback
    /// 4. Exchanges the authorization code for tokens
    pub async fn browser_flow(&self) -> Result<OAuthTokenResponse, OAuthError> {
        let meta = self.discover().await?;

        let verifier = Self::generate_verifier();
        let challenge = Self::generate_challenge(&verifier);
        let state = Self::generate_state();

        // Bind a random port for the callback
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        // Build authorization URL
        let mut auth_url = url::Url::parse(&meta.authorization_endpoint)?;
        auth_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        info!("Opening browser for OAuth login...");
        println!("Opening browser to: {}", auth_url);
        println!("If browser doesn't open, visit the URL above manually.");

        if open::that(auth_url.as_str()).is_err() {
            warn!("Failed to open browser automatically");
        }

        let code = tokio::task::spawn_blocking(move || wait_for_callback(listener, &state))
            .await
            .map_err(|e| OAuthError::TokenExchange(format!("callback task failed: {e}")))??;

        // Exchange code for tokens
        self.exchange_code(&code, &verifier, &redirect_uri, &meta.token_endpoint)
            .await
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        token_endpoint: &str,
    ) -> Result<OAuthTokenResponse, OAuthError> {
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &self.client_id),
            ("code_verifier", verifier),
        ];

        let resp = self.http.post(token_endpoint).form(&params).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!(
                "token endpoint returned {status}: {body}"
            )));
        }

        Ok(resp.json().await?)
    }
}

/// Wait for the OAuth redirect on the loopback listener.
///
/// Loops over connections with a 120-second deadline; non-/callback requests
/// (favicon, preflight, etc.) get a 404 and are ignored. The `state` parameter
/// is verified before the authorization code is accepted.
fn wait_for_callback(listener: TcpListener, state: &str) -> Result<String, OAuthError> {
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(OAuthError::TokenExchange(
                "OAuth callback timed out after 120s".into(),
            ));
        }

        // Accept a connection (non-blocking)
        let (mut stream, _) = match listener.accept() {
            Ok(conn) => conn,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(OAuthError::Io(e)),
        };

        // Read the request with a short timeout so broken connections don't hang us
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let n = match std::io::Read::read(&mut stream, &mut buf) {
            Ok(n) => n,
            Err(_) => continue, // Broken connection, try again
        };
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse the request line
        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("");

        // Ignore non-callback paths (favicon, preflight, etc.)
        if !path.starts_with("/callback") {
            let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            continue;
        }

        // Parse query parameters from the callback
        let url = url::Url::parse(&format!("http://localhost{path}")).map_err(OAuthError::Url)?;
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();

        // Verify state
        let received_state = params.get("state").cloned().unwrap_or_default();
        if received_state != state {
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><body><h1>State mismatch!</h1></body></html>";
            stream.write_all(response.as_bytes())?;
            return Err(OAuthError::TokenExchange("state mismatch".into()));
        }

        // Extract the authorization code
        let code = params
            .get("code")
            .ok_or_else(|| {
                OAuthError::TokenExchange(
                    params
                        .get("error_description")
                        .or(params.get("error"))
                        .cloned()
                        .unwrap_or_else(|| "no code in callback".into()),
                )
            })?
            .clone();

        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Login successful!</h1><p>You can close this tab and return to the terminal.</p></body></html>";
        stream.write_all(response.as_bytes())?;

        return Ok(code);
    }
}
