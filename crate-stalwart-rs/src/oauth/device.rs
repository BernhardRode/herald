//! Device Authorization Grant flow (RFC 8628).

use std::io::Write;

use super::{DeviceAuthResponse, OAuthError, OAuthTokenResponse, StalwartOAuth};

impl StalwartOAuth {
    /// Perform the OAuth2 Device Authorization Grant flow.
    ///
    /// 1. Discovers OAuth metadata (needs device_authorization_endpoint)
    /// 2. Requests a device code
    /// 3. Displays the user code and verification URI
    /// 4. Polls the token endpoint until approved or expired
    pub async fn device_flow(&self) -> Result<OAuthTokenResponse, OAuthError> {
        let meta = self.discover().await?;

        let device_endpoint = meta.device_authorization_endpoint.as_ref().ok_or_else(|| {
            OAuthError::Discovery("server does not advertise device_authorization_endpoint".into())
        })?;

        // Request device code
        let params = [("client_id", self.client_id.as_str())];

        let resp = self.http.post(device_endpoint).form(&params).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchange(format!(
                "device authorization returned {status}: {body}"
            )));
        }

        let device_resp: DeviceAuthResponse = resp.json().await?;

        // Display instructions to user
        println!();
        println!("╔══════════════════════════════════════════════════╗");
        println!("║           Device Authorization Flow              ║");
        println!("╠══════════════════════════════════════════════════╣");
        println!("║                                                  ║");
        println!("║  Visit: {:<40} ║", device_resp.verification_uri);
        println!("║  Code:  {:<40} ║", device_resp.user_code);
        println!("║                                                  ║");
        println!("╚══════════════════════════════════════════════════╝");
        println!();

        if let Some(ref complete_uri) = device_resp.verification_uri_complete {
            println!("Or open: {complete_uri}");
            let _ = open::that(complete_uri);
        }

        println!("Waiting for authorization...");

        // Poll token endpoint
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(device_resp.expires_in);
        let mut interval = std::time::Duration::from_secs(device_resp.interval);

        loop {
            tokio::time::sleep(interval).await;

            if std::time::Instant::now() > deadline {
                return Err(OAuthError::DeviceFlowExpired);
            }

            let params = [
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_resp.device_code),
                ("client_id", &self.client_id),
            ];

            let resp = self
                .http
                .post(&meta.token_endpoint)
                .form(&params)
                .send()
                .await?;

            if resp.status().is_success() {
                let tokens: OAuthTokenResponse = resp.json().await?;
                println!("✓ Authorization successful!");
                return Ok(tokens);
            }

            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let error = body["error"].as_str().unwrap_or("unknown");

            match error {
                "authorization_pending" => {
                    // Keep polling
                    print!(".");
                    let _ = std::io::stdout().flush();
                }
                "slow_down" => {
                    interval += std::time::Duration::from_secs(5);
                }
                "expired_token" => {
                    return Err(OAuthError::DeviceFlowExpired);
                }
                "access_denied" => {
                    return Err(OAuthError::TokenExchange("access denied by user".into()));
                }
                _ => {
                    return Err(OAuthError::TokenExchange(format!(
                        "device flow error: {error}"
                    )));
                }
            }
        }
    }
}
