//! Codex remote authorization: secrets live only in this one-shot request until commit.
use crate::{Agent, Mode, Session, Store};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hyper_util::client::proxy::matcher::Matcher;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};
use url::Url;

const CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT: &str = "http://localhost:1455/auth/callback";
const TOKEN: &str = "https://auth.openai.com/oauth/token";

/// One frozen route used both by the login preview and the token request.
/// The transport carries proxy credentials privately; summaries never print them.
pub struct LoginRoute {
    transport: Option<reqwest::Proxy>,
    summary: String,
    official_empty: bool,
}
impl LoginRoute {
    pub fn resolve(official: &str) -> Result<Self> {
        let matcher = if official.trim().is_empty() {
            Matcher::from_system()
        } else {
            Matcher::builder().build()
        };
        Self::from_matcher(official, matcher)
    }
    fn from_matcher(official: &str, matcher: Matcher) -> Result<Self> {
        let official = crate::config::normalize_proxy(official)?;
        if !official.is_empty() {
            return Ok(Self {
                transport: Some(reqwest::Proxy::all(&official).context("官方代理配置无效")?),
                summary: format!("换令牌代理：{official}（已保存的 Codex 官方代理）"),
                official_empty: false,
            });
        }
        if let Some(intercept) = matcher.intercept(&TOKEN.parse().unwrap()) {
            let uri = intercept.uri().to_string();
            let mut transport = reqwest::Proxy::all(&uri)
                .map_err(|_| anyhow::anyhow!("环境或系统代理无效，请设置 Codex 官方代理"))?;
            if let Some(auth) = intercept.basic_auth() {
                transport = transport.custom_http_auth(auth.clone());
            }
            if let Some((user, password)) = intercept.raw_auth() {
                transport = transport.basic_auth(user, password);
            }
            return Ok(Self {
                transport: Some(transport),
                summary: format!("换令牌代理：{uri}（继承环境 / 系统代理）"),
                official_empty: true,
            });
        }
        Ok(Self {
            transport: None,
            summary: "换令牌代理：直连".into(),
            official_empty: true,
        })
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn warning(&self) -> Option<&'static str> {
        self.official_empty.then_some("注意：官方代理未设置，直连可能因地区限制失败；当前实际路线以上方说明为准。可在登录确认按 p 设置代理，或使用 acs proxy --agent codex http://主机:端口。")
    }
}

// Intentionally no Debug/Serialize: neither PKCE verifier nor credentials belong in logs.
pub struct Login {
    session: Session,
    id: String,
    state: String,
    verifier: String,
    route: LoginRoute,
}

impl Login {
    pub fn begin(store: Store, id: &str) -> Result<Self> {
        let session = Session::open(store)?;
        if !session
            .original
            .codex
            .library
            .official_accounts
            .iter()
            .any(|p| p.id == id)
        {
            bail!("登录仅支持 Codex 官方账号配置，请先新增或选择该类型配置")
        }
        let official = session
            .original
            .codex
            .library
            .official_proxy_url
            .as_deref()
            .unwrap_or_default();
        #[cfg(not(test))]
        let route = LoginRoute::resolve(official)?;
        #[cfg(test)]
        let route = LoginRoute::from_matcher(official, Matcher::builder().build())?;
        Ok(Self {
            session,
            id: id.into(),
            state: random_secret(),
            verifier: random_secret(),
            route,
        })
    }
    pub fn route(&self) -> &LoginRoute {
        &self.route
    }

    pub fn authorization_url(&self) -> String {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        let mut url = Url::parse("https://auth.openai.com/oauth/authorize").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("client_id", CLIENT),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT),
            ("scope", "openid profile email offline_access"),
            ("state", &self.state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
        ]);
        url.into()
    }

    fn code(&self, callback: &str) -> Result<String> {
        let u = Url::parse(callback.trim())
            .map_err(|_| anyhow::anyhow!("回调 URL 无效，请粘贴地址栏中的完整 URL"))?;
        if u.scheme() != "http"
            || u.host_str() != Some("localhost")
            || u.port() != Some(1455)
            || u.path() != "/auth/callback"
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
        {
            bail!("回调地址必须为 http://localhost:1455/auth/callback，并包含本次登录参数")
        }
        let one = |key: &str| -> Result<Option<String>> {
            let values: Vec<_> = u
                .query_pairs()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
                .collect();
            match values.as_slice() {
                [] => Ok(None),
                [v] if !v.is_empty() => Ok(Some(v.clone())),
                _ => bail!("回调参数 {key} 重复或为空，请重新登录"),
            }
        };
        if one("state")?.as_deref() != Some(&self.state) {
            bail!("state 不匹配：请使用本次授权链接产生的回调 URL，勿混用其他登录窗口")
        }
        if one("error")?.is_some() {
            bail!("OpenAI 未批准本次授权，请重新登录并允许授权")
        }
        one("code")?.context("回调 URL 缺少 code，请完成浏览器授权后再粘贴")
    }

    pub fn complete(self, callback: &str) -> Result<()> {
        self.complete_at(callback, TOKEN)
    }

    // Private endpoint injection is only used by unit tests; no runtime test bypass.
    fn complete_at(mut self, callback: &str, endpoint: &str) -> Result<()> {
        let code = self.code(callback)?;
        self.session.check(&self.session.view().revision)?;
        let mut builder = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60));
        if let Some(proxy) = self.route.transport {
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .map_err(|_| anyhow::anyhow!("无法创建 OAuth 网络客户端"))?;
        let response = client
            .post(endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT),
                ("redirect_uri", REDIRECT),
                ("code", &code),
                ("code_verifier", &self.verifier),
            ])
            .send()
            .map_err(|_| {
                anyhow::anyhow!("换取 token 失败：网络连接或代理异常，请检查网络后重新登录")
            })?;
        let status = response.status();
        let mut bytes = Vec::new();
        let read = response.take(1024 * 1024 + 1).read_to_end(&mut bytes);
        if !status.is_success() {
            if read.is_ok() && bytes.len() <= 1024 * 1024 && unsupported_region(&bytes) {
                bail!("换取 token 失败（HTTP {}）：OpenAI 不支持当前出口地区。请在登录确认框按 p 设置并保存官方代理；或执行 acs proxy --agent codex http://主机:端口，再重新登录。",status.as_u16())
            }
            bail!(
                "换取 token 失败（HTTP {}），授权码可能已失效或已使用，请重新登录",
                status.as_u16()
            )
        }
        read.map_err(|_| anyhow::anyhow!("读取 token 响应失败，请重新登录"))?;
        if bytes.len() > 1024 * 1024 {
            bail!("token 响应过大，未保存凭据")
        }
        let tokens: Tokens = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("token 响应格式无效或凭据缺失，未保存凭据"))?;
        let claims = tokens.claims()?;
        let account = claims["https://api.openai.com/auth"]["chatgpt_account_id"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .context("token 缺少 Codex 账号 ID，未保存凭据")?;
        let mut draft = self.session.original.clone();
        let p = draft
            .codex
            .library
            .official_accounts
            .iter_mut()
            .find(|p| p.id == self.id)
            .context("登录配置已不存在")?;
        p.account_id = Some(account.into());
        p.email = claims["email"].as_str().map(str::to_owned);
        p.account_name = claims["name"].as_str().map(str::to_owned);
        p.official_auth = Some(serde_json::to_string(&json!({
            "auth_mode":"chatgpt", "OPENAI_API_KEY":null,
            "tokens":{"id_token":tokens.id_token,"access_token":tokens.access_token,"refresh_token":tokens.refresh_token,"account_id":account},
            "last_refresh":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }))?);
        draft.codex.mode = Mode::Official;
        draft.codex.library.select(Mode::Official, self.id);
        self.session
            .save_for(&self.session.view().revision, &draft, Agent::Codex)?;
        Ok(())
    }
}
fn unsupported_region(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    [
        "unsupported_country_region_territory",
        "country_not_supported",
        "unsupported_country",
        "region_not_supported",
        "unsupported_region",
        "country, region, or territory not supported",
        "unsupported country",
        "region is not supported",
    ]
    .iter()
    .any(|code| text.contains(code))
}

fn random_secret() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Deserialize)]
struct Tokens {
    id_token: String,
    access_token: String,
    refresh_token: String,
}
impl Tokens {
    fn claims(&self) -> Result<Value> {
        if [&self.id_token, &self.access_token, &self.refresh_token]
            .iter()
            .any(|s| s.trim().is_empty())
        {
            bail!("token 响应凭据为空，未保存凭据")
        }
        // Identity labels only; TLS token endpoint is the trust boundary, not unverified JWT claims.
        self.id_token
            .split('.')
            .nth(1)
            .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .context("id_token 格式无效，未保存凭据")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{storage::CODEX_AUTH, Roots};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn fixture() -> (tempfile::TempDir, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Roots::under(dir.path()));
        let mut s = Session::open(store.clone()).unwrap();
        let mut d = s.original.clone();
        let id = d
            .codex
            .add(Mode::Official, "example account", None)
            .unwrap();
        s.save_for(&s.view().revision, &d, Agent::Codex).unwrap();
        (dir, store, id)
    }
    fn callback(login: &Login) -> String {
        format!("{REDIRECT}?state={}&code=example-code", login.state)
    }
    fn valid_response() -> String {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"example@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"example-account"}}"#);
        json!({"id_token":format!("example.{payload}.signature"),"access_token":"example-access","refresh_token":"example-refresh"}).to_string()
    }
    #[test]
    fn route_discovery_describes_direct_environment_no_proxy_and_explicit_override() {
        let direct = LoginRoute::from_matcher("", Matcher::builder().build()).unwrap();
        assert_eq!(direct.summary(), "换令牌代理：直连");
        assert!(direct.warning().unwrap().contains("地区限制"));
        let inherited = LoginRoute::from_matcher(
            "",
            Matcher::builder()
                .https("http://proxy.example.test:8080")
                .build(),
        )
        .unwrap();
        assert!(
            inherited.summary().contains("proxy.example.test:8080")
                && inherited.summary().contains("继承")
        );
        let excluded = LoginRoute::from_matcher(
            "",
            Matcher::builder()
                .https("http://proxy.example.test:8080")
                .no(".openai.com")
                .build(),
        )
        .unwrap();
        assert_eq!(excluded.summary(), "换令牌代理：直连");
        let saved = LoginRoute::from_matcher(
            "http://official.example.test:8081",
            Matcher::builder()
                .https("http://environment.example.test:8080")
                .no("*")
                .build(),
        )
        .unwrap();
        assert!(saved.summary().contains("official.example.test:8081"));
        assert!(saved.warning().is_none());
        let credentials = LoginRoute::from_matcher(
            "",
            Matcher::builder()
                .https("http://example-user:example-private-password@proxy.example.test:8080")
                .build(),
        )
        .unwrap();
        assert!(!credentials.summary().contains("example-private-password"));
    }
    #[test]
    fn saved_official_proxy_is_the_actual_token_transport_and_region_errors_are_actionable() {
        let (_dir, store, id) = fixture();
        let (proxy, handle) = server(200, valid_response(), |request| {
            assert!(request.starts_with("POST http://token.example.test/oauth/token HTTP/1.1"));
            assert!(request.contains("code_verifier="));
        });
        let mut s = Session::open(store.clone()).unwrap();
        let mut draft = s.original.clone();
        draft.codex.library.official_proxy_url = Some(proxy.trim_end_matches("/token").into());
        s.save_for(&s.view().revision, &draft, Agent::Codex)
            .unwrap();
        let login = Login::begin(store.clone(), &id).unwrap();
        assert!(login.route().summary().contains("已保存"));
        let cb = callback(&login);
        login
            .complete_at(&cb, "http://token.example.test/oauth/token")
            .unwrap();
        handle.join().unwrap();
        for (body, regional) in [
            (
                r#"{"error":{"code":"unsupported_country_region_territory","message":"example-private-response"}}"#,
                true,
            ),
            (
                r#"{"error":{"code":"invalid_grant","message":"example-private-response"}}"#,
                false,
            ),
        ] {
            let (proxy, handle) = server(403, body.into(), |_| {});
            let mut s = Session::open(store.clone()).unwrap();
            let mut draft = s.original.clone();
            draft.codex.library.official_proxy_url = Some(proxy.trim_end_matches("/token").into());
            s.save_for(&s.view().revision, &draft, Agent::Codex)
                .unwrap();
            let before = store.snapshot().unwrap();
            let login = Login::begin(store.clone(), &id).unwrap();
            let cb = callback(&login);
            let error = login
                .complete_at(&cb, "http://token.example.test/oauth/token")
                .unwrap_err()
                .to_string();
            handle.join().unwrap();
            assert!(!error.contains("example-private-response"));
            assert_eq!(error.contains("acs proxy --agent codex"), regional);
            if regional {
                assert!(error.contains("登录确认框按 p") && error.contains("出口地区"));
            } else {
                assert!(error.contains("授权码可能已失效"));
            }
            assert_eq!(store.snapshot().unwrap(), before);
        }
    }
    fn server(
        status: u16,
        body: String,
        verify: impl FnOnce(&str) + Send + 'static,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("[::1]:0")
            .or_else(|_| TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..at]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= at + 4 + length {
                        break;
                    }
                }
            }
            verify(&String::from_utf8(bytes).unwrap());
            write!(socket,"HTTP/1.1 {status} Example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        (endpoint, handle)
    }

    #[test]
    fn authorization_is_random_pkce_and_callback_is_strict() {
        let (_dir, store, id) = fixture();
        let login = Login::begin(store.clone(), &id).unwrap();
        let next = Login::begin(store, &id).unwrap();
        assert_ne!(login.state, next.state);
        assert_ne!(login.verifier, next.verifier);
        let url = Url::parse(&login.authorization_url()).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], CLIENT);
        assert_eq!(q["redirect_uri"], REDIRECT);
        assert_eq!(q["scope"], "openid profile email offline_access");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(
            q["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.as_bytes()))
        );
        assert!(!login.authorization_url().contains(&login.verifier));
        assert_eq!(login.code(&callback(&login)).unwrap(), "example-code");
        for bad in [
            "garbage".into(),
            callback(&next),
            callback(&login).replace("localhost", "example.com"),
            callback(&login).replace(":1455", ":1456"),
            format!("{}&code=duplicate", callback(&login)),
            format!("{}&state=duplicate", callback(&login)),
            format!("{}#fragment", callback(&login)),
            format!("{REDIRECT}?state={}&error=access_denied", login.state),
            format!("{REDIRECT}?state={}", login.state),
        ] {
            assert!(login.code(&bad).is_err());
        }
    }
    #[test]
    fn exchanges_form_and_atomically_saves_official_credentials() {
        let (_dir, store, id) = fixture();
        let login = Login::begin(store.clone(), &id).unwrap();
        let cb = callback(&login);
        let verifier = login.verifier.clone();
        let (endpoint, handle) = server(200, valid_response(), move |request| {
            assert!(request.starts_with("POST /token HTTP/1.1"));
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let form: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect();
            assert_eq!(form["code_verifier"], verifier);
            assert_eq!(form["code"], "example-code");
            assert_eq!(form["redirect_uri"], REDIRECT);
            assert_eq!(form["grant_type"], "authorization_code");
        });
        login.complete_at(&cb, &endpoint).unwrap();
        handle.join().unwrap();
        let snapshot = store.snapshot().unwrap();
        let auth: Value = serde_json::from_slice(snapshot[CODEX_AUTH].as_ref().unwrap()).unwrap();
        assert!(crate::codex::valid_tokens(&auth));
        assert!(auth["last_refresh"].as_str().unwrap().ends_with('Z'));
        let s = Session::open(store).unwrap();
        assert_eq!(s.original.codex.library.selected_official, id);
        assert_eq!(
            s.original
                .codex
                .library
                .selected(Mode::Official)
                .unwrap()
                .email
                .as_deref(),
            Some("example@example.com")
        );
    }
    #[test]
    fn failure_and_external_change_leave_all_targets_unchanged_and_hide_response_secrets() {
        let (_dir, store, id) = fixture();
        for (status, body) in [
            (400, "example-private-code-token".into()),
            (200, "malformed example-private-code-token".into()),
            (
                200,
                json!({"id_token":"bad","access_token":"a","refresh_token":"r"}).to_string(),
            ),
            (
                200,
                json!({"id_token":"bad","access_token":"","refresh_token":"r"}).to_string(),
            ),
        ] {
            let login = Login::begin(store.clone(), &id).unwrap();
            let cb = callback(&login);
            let before = store.snapshot().unwrap();
            let (endpoint, handle) = server(status, body, |_| {});
            let error = login.complete_at(&cb, &endpoint).unwrap_err();
            handle.join().unwrap();
            assert!(!format!("{error:#}").contains("example-private"));
            assert_eq!(store.snapshot().unwrap(), before);
        }
        let login = Login::begin(store.clone(), &id).unwrap();
        let cb = callback(&login);
        crate::storage::write(&store.roots.codex.join(".env"), Some(b"EXAMPLE=changed")).unwrap();
        let before = store.snapshot().unwrap();
        assert!(login.complete_at(&cb, "http://127.0.0.1:1/token").is_err());
        assert_eq!(store.snapshot().unwrap(), before);
        let login = Login::begin(store.clone(), &id).unwrap();
        assert!(login
            .complete_at(
                "invalid callback containing example-private-code-token",
                "http://127.0.0.1:1"
            )
            .is_err());
        assert_eq!(store.snapshot().unwrap(), before);
    }

    #[test]
    fn changes_during_exchange_and_save_failures_do_not_commit_tokens() {
        let (_dir, store, id) = fixture();
        let login = Login::begin(store.clone(), &id).unwrap();
        let cb = callback(&login);
        let remote = store.clone();
        let (endpoint, handle) = server(200, valid_response(), move |_| {
            crate::storage::write(&remote.roots.codex.join(".env"), Some(b"EXAMPLE=external"))
                .unwrap();
        });
        assert!(login.complete_at(&cb, &endpoint).is_err());
        handle.join().unwrap();
        assert!(store.snapshot().unwrap()[CODEX_AUTH].is_none());
        assert!(Session::open(store.clone())
            .unwrap()
            .original
            .codex
            .library
            .selected(Mode::Official)
            .unwrap()
            .official_auth
            .is_none());

        // A directory at auth.json simulates a target replacement failure after exchange.
        // Existing storage tests separately inject every multi-file write failure and assert rollback.
        let login = Login::begin(store.clone(), &id).unwrap();
        let cb = callback(&login);
        let remote = store.clone();
        let (endpoint, handle) = server(200, valid_response(), move |_| {
            std::fs::create_dir(remote.roots.codex.join("auth.json")).unwrap();
        });
        assert!(login.complete_at(&cb, &endpoint).is_err());
        handle.join().unwrap();
        let library = std::fs::read_to_string(store.roots.acs.join("profiles.json")).unwrap();
        assert!(!library.contains("example-access"));
    }

    #[test]
    fn logging_into_same_account_keeps_the_explicit_target_selected_on_reopen() {
        let (_dir, store, first) = fixture();
        let mut ids = vec![first];
        let mut session = Session::open(store.clone()).unwrap();
        let mut draft = session.original.clone();
        ids.push(
            draft
                .codex
                .add(Mode::Official, "example second account", None)
                .unwrap(),
        );
        session
            .save_for(&session.view().revision, &draft, Agent::Codex)
            .unwrap();
        for id in ids {
            let login = Login::begin(store.clone(), &id).unwrap();
            let cb = callback(&login);
            let (endpoint, handle) = server(200, valid_response(), |_| {});
            login.complete_at(&cb, &endpoint).unwrap();
            handle.join().unwrap();
            assert_eq!(
                Session::open(store.clone())
                    .unwrap()
                    .original
                    .codex
                    .library
                    .selected_official,
                id
            );
        }
    }
}
