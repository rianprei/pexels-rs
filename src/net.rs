//! HTTP sobre libcurl (HTTP/2, gzip/brotli/zstd, conexões reutilizadas por `Client`): busca de página e download em fluxo.
//! libcurl, e não um cliente TLS próprio: a borda da Pexels responde 403 a clientes cujo aperto de mão TLS não reconhece.
use crate::error::{Error, Result};
use curl::easy::{Easy, List};
use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// Teto de uma página de busca (as reais têm de 60 kB a 1 MB): resposta maior que isso não é página de busca.
const MAX_PAGE: usize = 8 << 20;
/// Únicos hosts de onde se baixa mídia: a URL vem de conteúdo externo.
const MEDIA_HOSTS: [&str; 2] = ["videos.pexels.com", "images.pexels.com"];

/// Um handle de libcurl: requisições seguidas pelo mesmo `Client` reaproveitam a conexão (e a sessão TLS).
pub(crate) struct Client(Easy);

impl Client {
    pub(crate) fn new() -> Result<Self> {
        let mut e = Easy::new();
        e.useragent(UA)?;
        e.accept_encoding("")?; // "" = todas as codificações que a libcurl sabe descomprimir
        e.follow_location(true)?;
        e.max_redirections(3)?;
        e.connect_timeout(Duration::from_secs(15))?;
        e.low_speed_limit(1024)?; // abaixo de 1 kB/s por 30 s: conexão morta
        e.low_speed_time(Duration::from_secs(30))?;
        Ok(Self(e))
    }

    fn status(&self, what: &'static str) -> Result<()> {
        match self.0.response_code()? {
            200..=299 => Ok(()),
            code => Err(Error::Status(code, what)),
        }
    }

    /// Corpo de uma página, já descomprimido. `auth` só é enviado à API oficial.
    pub(crate) fn get_page(&mut self, url: &str, auth: Option<&str>) -> Result<Vec<u8>> {
        let mut headers = List::new();
        headers.append("Accept: text/html,application/json;q=0.9")?;
        headers.append("Accept-Language: en-US,en;q=0.9")?;
        if let Some(key) = auth {
            headers.append(&format!("Authorization: {key}"))?;
        }
        self.0.url(url)?;
        self.0.http_headers(headers)?;
        self.0.timeout(Duration::from_secs(60))?;
        let mut body = Vec::with_capacity(1 << 20);
        let mut too_big = false;
        {
            let mut t = self.0.transfer();
            t.write_function(|chunk| {
                if body.len() + chunk.len() > MAX_PAGE {
                    too_big = true;
                    return Ok(0); // aborta a transferência
                }
                body.extend_from_slice(chunk);
                Ok(chunk.len())
            })?;
            let done = t.perform();
            drop(t);
            if too_big {
                return Err(Error::Layout("resposta maior que 8 MB"));
            }
            done?;
        }
        self.status("busca na Pexels")?;
        Ok(body)
    }

    /// Baixa `url` para `dest`. Grava em `<dest>.part` e renomeia no fim: nunca fica arquivo pela metade com o nome final.
    /// Consome o cliente: o download muda opções do handle (sem redirecionamento, falha em 4xx/5xx) que não servem a buscas.
    pub(crate) fn download(mut self, url: &str, dest: &str) -> Result<u64> {
        match https_host(url) {
            Some(h) if MEDIA_HOSTS.contains(&h) => {}
            _ => {
                return Err(Error::Usage(format!(
                    "só baixo https de {}",
                    MEDIA_HOSTS.join(" ou ")
                )))
            }
        }
        let part = format!("{dest}.part");
        let mut out = BufWriter::with_capacity(
            1 << 18,
            File::create(&part).map_err(|e| Error::Io(part.clone(), e))?,
        );
        let result = self.download_into(url, &mut out).and_then(|n| {
            out.flush()
                .map(|()| n)
                .map_err(|e| Error::Io(part.clone(), e))
        });
        drop(out);
        match result {
            Ok(n) => fs::rename(&part, dest)
                .map(|()| n)
                .map_err(|e| Error::Io(dest.to_owned(), e)),
            Err(e) => {
                let _ = fs::remove_file(&part);
                Err(e)
            }
        }
    }

    fn download_into(&mut self, url: &str, out: &mut impl Write) -> Result<u64> {
        self.0.url(url)?;
        self.0.http_headers(List::new())?;
        self.0.max_redirections(0)?; // mídia vem direto do CDN; redirecionar poderia sair da lista de hosts
        self.0.timeout(Duration::from_secs(900))?;
        self.0.fail_on_error(true)?; // 4xx/5xx não viram arquivo
        let (mut written, mut io_failed) = (0usize, None);
        {
            let mut t = self.0.transfer();
            t.write_function(|chunk| match out.write_all(chunk) {
                Ok(()) => {
                    written += chunk.len();
                    Ok(chunk.len())
                }
                Err(e) => {
                    io_failed = Some(e);
                    Ok(0)
                }
            })?;
            let done = t.perform();
            drop(t);
            if let Some(e) = io_failed {
                return Err(Error::Io("gravação".into(), e));
            }
            // libcurl já acusa corpo menor que o Content-Length (erro 18) como falha
            if let Err(e) = done {
                return Err(if e.is_http_returned_error() {
                    Error::Status(self.0.response_code().unwrap_or(0), "download")
                } else {
                    Error::Curl(e)
                });
            }
        }
        Ok(u64::try_from(written).unwrap_or(u64::MAX))
    }
}

/// Cache em disco das páginas: a mesma busca repetida (o pipeline testa vários candidatos da mesma consulta) não volta ao site.
pub(crate) struct Cache {
    dir: PathBuf,
    ttl: Duration,
}

impl Cache {
    /// `~/.cache/pexels-rs` (ou `$XDG_CACHE_HOME/pexels-rs`). `None` se não houver onde gravar.
    pub(crate) fn open(ttl: Duration) -> Option<Self> {
        let base = env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        let dir = base.join("pexels-rs");
        fs::create_dir_all(&dir).ok()?;
        Some(Self { dir, ttl })
    }

    #[cfg(test)]
    pub(crate) const fn at(dir: PathBuf, ttl: Duration) -> Self {
        Self { dir, ttl }
    }

    fn path(&self, url: &str) -> PathBuf {
        // FNV-1a de 64 bits: só precisa espalhar bem as URLs em nomes de arquivo
        let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        self.dir.join(format!("{hash:016x}.html"))
    }

    pub(crate) fn get(&self, url: &str) -> Option<Vec<u8>> {
        let path = self.path(url);
        let age = fs::metadata(&path).ok()?.modified().ok()?.elapsed().ok()?;
        if age > self.ttl {
            return None;
        }
        fs::read(path).ok()
    }

    /// Falha de gravação não é erro da busca: o cache é só economia.
    pub(crate) fn put(&self, url: &str, body: &[u8]) {
        let path = self.path(url);
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&tmp, body)
            .and_then(|()| fs::rename(&tmp, &path))
            .is_err()
        {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// Esperas antes de repetir uma página recusada por limite ou falha passageira (403 de rajada, 429, 5xx, rede).
const BACKOFF: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(8)];

const fn retryable(e: &Error) -> bool {
    matches!(e, Error::Status(403 | 429 | 500..=599, _) | Error::Curl(_))
}

impl Client {
    /// Página pelo cache ou pela rede, com novas tentativas espaçadas. Devolve também se veio da rede (quem chama espaça as próximas).
    pub(crate) fn page(
        &mut self,
        url: &str,
        auth: Option<&str>,
        cache: Option<&Cache>,
    ) -> Result<(Vec<u8>, bool)> {
        if let Some(body) = cache.and_then(|c| c.get(url)) {
            return Ok((body, false));
        }
        let mut attempt = 0;
        let body = loop {
            match self.get_page(url, auth) {
                Ok(b) => break b,
                Err(e) if attempt < BACKOFF.len() && retryable(&e) => {
                    std::thread::sleep(BACKOFF[attempt]);
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        };
        if let Some(c) = cache {
            c.put(url, &body);
        }
        Ok((body, true))
    }
}

/// Host de uma URL https, ou `None` se não for https bem formada.
pub(crate) fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    (!host.is_empty() && !host.contains(['@', ':'])).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing_is_strict() {
        assert_eq!(
            https_host("https://videos.pexels.com/video-files/1/a.mp4"),
            Some("videos.pexels.com")
        );
        assert_eq!(
            https_host("https://images.pexels.com?x=1"),
            Some("images.pexels.com")
        );
        assert_eq!(https_host("http://videos.pexels.com/a.mp4"), None);
        assert_eq!(
            https_host("https://videos.pexels.com@evil.example/a.mp4"),
            None
        );
        assert_eq!(https_host("https://evil.example:443/a"), None);
        assert_eq!(https_host("file:///etc/passwd"), None);
    }

    #[test]
    fn download_refuses_other_hosts_before_any_request() {
        for url in [
            "https://evil.example/a.mp4",
            "http://videos.pexels.com/a.mp4",
            "https://videos.pexels.com.evil.example/a.mp4",
            "file:///etc/passwd",
        ] {
            assert!(
                matches!(
                    Client::new()
                        .expect("handle da libcurl")
                        .download(url, "/nonexistent/x"),
                    Err(Error::Usage(_))
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn cache_round_trips_and_expires() {
        let dir = env::temp_dir().join(format!("pexels-rs-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let fresh = Cache::at(dir.clone(), Duration::from_secs(60));
        assert_eq!(fresh.get("https://www.pexels.com/search/ice/"), None);
        fresh.put("https://www.pexels.com/search/ice/", b"<html>ice</html>");
        assert_eq!(
            fresh.get("https://www.pexels.com/search/ice/").as_deref(),
            Some(&b"<html>ice</html>"[..])
        );
        assert_eq!(
            fresh.get("https://www.pexels.com/search/ice/?page=2"),
            None,
            "outra URL, outra entrada"
        );
        assert_eq!(
            Cache::at(dir.clone(), Duration::ZERO).get("https://www.pexels.com/search/ice/"),
            None,
            "vencido"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_passing_failures_are_retried() {
        assert!(
            retryable(&Error::Status(403, "x"))
                && retryable(&Error::Status(429, "x"))
                && retryable(&Error::Status(503, "x"))
                && retryable(&Error::Curl(curl::Error::new(7)))
        );
        assert!(
            !retryable(&Error::Status(404, "x"))
                && !retryable(&Error::Status(401, "x"))
                && !retryable(&Error::Layout("x"))
                && !retryable(&Error::Usage("x".into()))
        );
    }
}
