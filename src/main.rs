//! pexels-rs: busca e download de vídeos e fotos da Pexels, pelas páginas públicas (sem chave) ou pela API oficial (`--api`).
mod api;
mod error;
mod model;
mod net;
mod web;

use error::{Error, Result};
use model::{Filter, Pick};
use net::{Cache, Client};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, BufWriter, Write};
use std::{env, fs, process::ExitCode, thread, time::Duration};
use web::{Kind, Listing, Server, Shape};

const USAGE: &str = "\
pexels-rs: vídeos e fotos da Pexels sem chave de API (ou com chave, usando --api)

BUSCA E LISTAS (uma linha JSON por resultado)
  pexels-rs videos <consulta>            busca de vídeos
  pexels-rs photos <consulta>            busca de fotos
  pexels-rs popular                      vídeos populares do momento (18; com --api, paginado)
  pexels-rs curated                      fotos da curadoria (12; com --api, paginado)
  pexels-rs collection <slug>            mídia de uma coleção (os 12 primeiros itens)
  pexels-rs user <usuario>               mídia de um autor
  pexels-rs similar video|photo <id>     mídia parecida com uma mídia
DETALHE
  pexels-rs get video|photo <id>         uma mídia com todas as rendições, visualizações, downloads, curtidas e detalhes (EXIF)
  pexels-rs related <consulta>           buscas relacionadas, com a quantidade de resultados de cada uma
  pexels-rs collections                  coleções em destaque
ARQUIVOS
  pexels-rs download <url> <arquivo>     baixa de videos.pexels.com ou images.pexels.com
  pexels-rs parse list|detail|collections <arquivo.html>   lê uma página salva (offline)

FILTROS DO SITE (videos, photos)
  --orientation portrait|landscape|square   --color RRGGBB (fotos)        --people N   --age <faixa>
  --min-duration S   --max-duration S (vídeos)                             --date-from <período, ex.: last_year>
FILTROS LOCAIS (qualquer lista)
  --min-fps N   --min-width N   --with <palavra> (repetível)   --without <palavra> (repetível)   --only videos|photos
  --shorter-than S   --longer-than S        (duração, onde o site não filtra: coleções, autores, populares)
ESCOLHA E SAÍDA
  --limit N           segue paginando até N resultados (padrão: uma página)       --page N   página inicial
  --min-height N      altura mínima do arquivo de vídeo                           --target-short N   lado menor preferido (padrão 1080)
  --photo-width N     largura da foto entregue pelo CDN (padrão 2160)             --files   todas as rendições de cada vídeo
REDE
  --api               usa a API oficial (exige PEXELS_API_KEY; vale para videos, photos, popular, curated)
  --no-cache          ignora o cache em disco      --cache-ttl S   validade do cache em segundos (padrão 21600)";

/// Intervalo entre páginas pedidas à rede na mesma execução: a borda da Pexels responde 403 a rajadas.
const PAUSE: Duration = Duration::from_millis(700);
/// Teto de páginas por execução, para um `--limit` grande não virar varredura do site.
const MAX_PAGES: u32 = 40;

struct Args<'a>(&'a [String]);

impl<'a> Args<'a> {
    fn value(&self, name: &str) -> Option<&'a str> {
        self.0
            .iter()
            .position(|a| a == name)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn values(&self, name: &str) -> Vec<String> {
        self.0
            .windows(2)
            .filter(|w| w[0] == name)
            .map(|w| w[1].to_lowercase())
            .collect()
    }
    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|a| a == name)
    }
    fn num<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        self.value(name)
            .map(|v| {
                v.parse()
                    .map_err(|_| Error::Usage(format!("{name} precisa ser um número, veio: {v}")))
            })
            .transpose()
    }
    /// Argumento posicional `i` (0 = subcomando). Um valor começando por `--` não é posicional.
    fn pos(&self, i: usize) -> Result<&'a str> {
        self.0
            .get(i)
            .map(String::as_str)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| Error::Usage(USAGE.into()))
    }
}

fn kind(s: &str) -> Result<Kind> {
    match s {
        "video" | "videos" => Ok(Kind::Videos),
        "photo" | "photos" => Ok(Kind::Photos),
        other => Err(Error::Usage(format!(
            "esperava video ou photo, veio: {other}"
        ))),
    }
}

/// De onde vêm os dados.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Backend {
    /// Páginas públicas do site, sem chave.
    Web,
    /// API oficial, com `PEXELS_API_KEY`.
    Api,
}

/// O que `get` e `similar` tiram da página de detalhe.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Detail {
    Media,
    Similar,
}

struct Opts<'a> {
    server: Server<'a>,
    shape: Shape,
    filter: Filter,
    page: u32,
    limit: Option<usize>,
    backend: Backend,
    cache: Option<Cache>,
}

fn opts<'a>(a: &Args<'a>) -> Result<Opts<'a>> {
    let orientation = a.value("--orientation");
    let mut pick = Pick::for_orientation(orientation);
    pick.min_height = a.num("--min-height")?.unwrap_or(pick.min_height);
    pick.target_short = a.num("--target-short")?.unwrap_or(pick.target_short);
    let only = match a.value("--only") {
        None => None,
        Some(v) => Some(if kind(v)? == Kind::Videos {
            "video"
        } else {
            "photo"
        }),
    };
    let cache = if a.has("--no-cache") {
        None
    } else {
        Cache::open(Duration::from_secs(a.num("--cache-ttl")?.unwrap_or(21_600)))
    };
    Ok(Opts {
        server: Server {
            orientation,
            color: a.value("--color"),
            people: a.value("--people"),
            age: a.value("--age"),
            date_from: a.value("--date-from"),
            min_duration: a.num("--min-duration")?,
            max_duration: a.num("--max-duration")?,
        },
        shape: Shape {
            pick,
            photo_width: a.num("--photo-width")?.unwrap_or(2160u32).clamp(320, 6000),
            all_files: a.has("--files"),
        },
        filter: Filter {
            only,
            min_width: a.num("--min-width")?.unwrap_or(0),
            min_fps: a.num("--min-fps")?.unwrap_or(0.0),
            min_duration: a.num("--longer-than")?.unwrap_or(0.0),
            max_duration: a.num("--shorter-than")?.unwrap_or(0.0),
            with: a.values("--with"),
            without: a.values("--without"),
        },
        page: a.num("--page")?.unwrap_or(1u32).max(1),
        limit: a.num("--limit")?,
        backend: if a.has("--api") {
            Backend::Api
        } else {
            Backend::Web
        },
        cache,
    })
}

fn stdout_err(e: io::Error) -> Error {
    Error::Io("stdout".into(), e)
}

fn emit<T: Serialize>(out: &mut impl Write, row: &T) -> Result<()> {
    serde_json::to_writer(&mut *out, row)?;
    out.write_all(b"\n").map_err(stdout_err)
}

/// De onde vem cada página de uma lista.
enum Source<'a> {
    Search(Kind, &'a str),
    Feed(Kind),
    Collection(&'a str),
    User(&'a str),
}

impl Source<'_> {
    /// Populares e curadoria usam cursor, e a coleção ignora `page` (medido em 17/11/2026: a página 2 repete a 1):
    /// no HTML público essas três só expõem a primeira página (18, 12 e 12 itens).
    fn paged(&self, backend: Backend) -> bool {
        backend == Backend::Api || matches!(self, Source::Search(..) | Source::User(_))
    }

    fn url(&self, page: u32, o: &Opts<'_>) -> Result<String> {
        match (self, o.backend) {
            (Source::Search(k, q), Backend::Api) => {
                api::search_url(*k, q, page, 80, o.server.orientation)
            }
            (Source::Search(k, q), Backend::Web) => web::search_url(*k, q, page, &o.server),
            (Source::Feed(k), Backend::Api) => Ok(api::feed_url(*k, page, 80)),
            (Source::Feed(Kind::Videos), Backend::Web) => Ok(web::POPULAR_VIDEOS.into()),
            (Source::Feed(Kind::Photos), Backend::Web) => Ok(web::CURATED_PHOTOS.into()),
            (Source::Collection(c), _) => web::collection_url(c, page),
            (Source::User(u), _) => web::user_url(u, page),
        }
    }

    fn kind(&self) -> Option<Kind> {
        match self {
            Source::Search(k, _) | Source::Feed(k) => Some(*k),
            _ => None,
        }
    }
}

/// Percorre as páginas de uma fonte e emite as linhas aceitas, sem repetir ids, até `--limit` (ou uma página, sem o limite).
fn list(source: &Source<'_>, o: &Opts<'_>) -> Result<()> {
    let api = o.backend == Backend::Api;
    let key = if api {
        if matches!(source, Source::Collection(_) | Source::User(_)) {
            return Err(Error::Usage(
                "--api só vale para videos, photos, popular e curated".into(),
            ));
        }
        Some(
            env::var("PEXELS_API_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| Error::Usage("--api exige PEXELS_API_KEY".into()))?,
        )
    } else {
        None
    };
    let key = key.as_deref().map(str::trim);
    let cache = if api { None } else { o.cache.as_ref() }; // respostas autenticadas não vão para o disco
    let want = o.limit.unwrap_or(usize::MAX);
    let mut client = Client::new()?;
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    let (mut seen, mut emitted, mut hit_network) = (HashSet::new(), 0usize, false);
    for page in o.page..o.page.saturating_add(MAX_PAGES) {
        let url = source.url(page, o)?;
        if hit_network {
            thread::sleep(PAUSE);
        }
        let (body, from_network) = client.page(&url, key, cache)?;
        hit_network = from_network;
        let (rows, items, total_pages) = if api {
            let rows = api::parse(
                &body,
                source.kind().unwrap_or(Kind::Videos),
                o.shape.pick,
                o.shape.photo_width,
            )?;
            let n = rows.len();
            (rows, n, u32::MAX)
        } else {
            let Listing {
                rows,
                items,
                pagination,
                ..
            } = web::parse_list(&body, &o.shape)?;
            (rows, items, pagination.map_or(1, |p| p.total_pages))
        };
        // uma página que só repete ids já vistos significa que a fonte parou de avançar
        let mut fresh = false;
        for r in &rows {
            if !seen.insert((r.kind, r.id)) {
                continue;
            }
            fresh = true;
            if emitted < want && o.filter.accepts(r) {
                emit(&mut out, r)?;
                emitted += 1;
            }
        }
        if emitted >= want || o.limit.is_none() || items == 0 || !fresh || page >= total_pages {
            break;
        }
        if !source.paged(o.backend) {
            eprintln!("pexels-rs: esta lista só expõe a primeira página no site; {emitted} resultado(s) de {want} pedidos");
            break;
        }
    }
    out.flush().map_err(stdout_err)
}

fn detail(k: Kind, id: &str, o: &Opts<'_>, what: Detail) -> Result<()> {
    let id: u64 = id
        .parse()
        .map_err(|_| Error::Usage(format!("id precisa ser numérico: {id}")))?;
    let (body, _) = Client::new()?.page(&web::media_url(k, id), None, o.cache.as_ref())?;
    // no detalhe, todas as rendições fazem parte da resposta
    let shape = Shape {
        all_files: what == Detail::Media || o.shape.all_files,
        ..o.shape
    };
    let (row, similar_rows) = web::parse_detail(&body, &shape)?;
    let mut out = BufWriter::new(io::stdout().lock());
    if what == Detail::Similar {
        for r in similar_rows
            .rows
            .iter()
            .filter(|r| o.filter.accepts(r))
            .take(o.limit.unwrap_or(usize::MAX))
        {
            emit(&mut out, r)?;
        }
    } else {
        emit(
            &mut out,
            &row.ok_or(Error::Layout(
                "mídia sem arquivo que atenda a --min-height/--orientation",
            ))?,
        )?;
    }
    out.flush().map_err(stdout_err)
}

fn parse_file(mode: &str, path: &str, o: &Opts<'_>) -> Result<()> {
    let html = fs::read(path).map_err(|e| Error::Io(path.to_owned(), e))?;
    let mut out = BufWriter::new(io::stdout().lock());
    match mode {
        "list" => {
            let l = web::parse_list(&html, &o.shape)?;
            if let Some(p) = l.pagination {
                eprintln!(
                    "página {} de {} ({} resultados)",
                    p.current_page, p.total_pages, p.total_results
                );
            }
            l.rows
                .iter()
                .filter(|r| o.filter.accepts(r))
                .try_for_each(|r| emit(&mut out, r))?;
        }
        "detail" => {
            let (row, similar) = web::parse_detail(
                &html,
                &Shape {
                    all_files: true,
                    ..o.shape
                },
            )?;
            row.iter()
                .chain(similar.rows.iter())
                .try_for_each(|r| emit(&mut out, r))?;
        }
        "collections" => web::parse_collections(&html)?
            .iter()
            .try_for_each(|c| emit(&mut out, c))?,
        other => {
            return Err(Error::Usage(format!(
                "parse: esperava list, detail ou collections, veio: {other}"
            )))
        }
    }
    out.flush().map_err(stdout_err)
}

fn run(argv: &[String]) -> Result<()> {
    let a = Args(argv);
    let cmd = a.pos(0)?;
    match cmd {
        "download" => {
            let bytes = Client::new()?.download(a.pos(1)?, a.pos(2)?)?;
            println!("{}", serde_json::json!({"file": a.pos(2)?, "bytes": bytes}));
            return Ok(());
        }
        "help" | "-h" => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => {}
    }
    let o = opts(&a)?;
    match cmd {
        "videos" | "photos" => list(&Source::Search(kind(cmd)?, a.pos(1)?), &o),
        "popular" => list(&Source::Feed(Kind::Videos), &o),
        "curated" => list(&Source::Feed(Kind::Photos), &o),
        "collection" => list(&Source::Collection(a.pos(1)?), &o),
        "user" => list(&Source::User(a.pos(1)?), &o),
        "get" => detail(kind(a.pos(1)?)?, a.pos(2)?, &o, Detail::Media),
        "similar" => detail(kind(a.pos(1)?)?, a.pos(2)?, &o, Detail::Similar),
        "related" => {
            let url = web::search_url(Kind::Videos, a.pos(1)?, 1, &Server::default())?;
            let (body, _) = Client::new()?.page(&url, None, o.cache.as_ref())?;
            let mut out = BufWriter::new(io::stdout().lock());
            web::parse_list(&body, &o.shape)?
                .related
                .iter()
                .try_for_each(|r| emit(&mut out, r))?;
            out.flush().map_err(stdout_err)
        }
        "collections" => {
            let (body, _) = Client::new()?.page(web::DISCOVER, None, o.cache.as_ref())?;
            let mut out = BufWriter::new(io::stdout().lock());
            web::parse_collections(&body)?
                .iter()
                .try_for_each(|c| emit(&mut out, c))?;
            out.flush().map_err(stdout_err)
        }
        "parse" => parse_file(a.pos(1)?, a.pos(2)?, &o),
        _ => Err(Error::Usage(USAGE.into())),
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pexels-rs: {e}");
            ExitCode::FAILURE
        }
    }
}
