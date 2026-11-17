//! Fonte sem chave: as páginas públicas da Pexels trazem os dados em JSON dentro de `<script id="__NEXT_DATA__">`.
//! Uma requisição por página, como um navegador faria. Sem login, sem credencial.
use crate::error::{Error, Result};
use crate::model::{FileRow, Pick, Row};
use memchr::memmem;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::borrow::Cow;

pub const SITE: &str = "https://www.pexels.com";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Videos,
    Photos,
}

/// Filtros que o próprio site aplica (viram parâmetros da URL de busca).
#[derive(Debug, Default, Clone, Copy)]
pub struct Server<'a> {
    pub orientation: Option<&'a str>,
    /// Cor dominante da foto: seis dígitos hexadecimais.
    pub color: Option<&'a str>,
    pub people: Option<&'a str>,
    pub age: Option<&'a str>,
    pub date_from: Option<&'a str>,
    pub min_duration: Option<u32>,
    pub max_duration: Option<u32>,
}

/// Percent-encoding (RFC 3986): só os caracteres não reservados passam.
pub fn encode_into(out: &mut String, s: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
    }
}

/// Valor de filtro vindo da linha de comando: só letras, dígitos e `_` chegam à URL.
fn word<'a>(name: &str, v: &'a str) -> Result<&'a str> {
    if !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        Ok(v)
    } else {
        Err(Error::Usage(format!("{name} inválido: {v}")))
    }
}

struct Query {
    url: String,
    sep: char,
}

impl Query {
    fn new(url: String) -> Self {
        Query { url, sep: '?' }
    }
    fn add(&mut self, key: &str, value: &str) {
        self.url.push(self.sep);
        self.url.push_str(key);
        self.url.push('=');
        self.url.push_str(value);
        self.sep = '&';
    }
    /// A página 1 com `page=1` responde 301 para a URL sem o parâmetro: pedir a forma canônica poupa uma ida e volta.
    fn page(mut self, page: u32) -> String {
        if page > 1 {
            self.add("page", &page.to_string());
        }
        self.url
    }
}

pub fn search_url(kind: Kind, query: &str, page: u32, f: &Server<'_>) -> Result<String> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::Usage("consulta vazia".into()));
    }
    let mut url = String::with_capacity(128 + query.len() * 3);
    url.push_str(SITE);
    url.push_str(if kind == Kind::Videos { "/search/videos/" } else { "/search/" });
    encode_into(&mut url, query);
    url.push('/');
    let mut q = Query::new(url);
    if let Some(o) = f.orientation {
        if !matches!(o, "portrait" | "landscape" | "square") {
            return Err(Error::Usage(format!("orientation inválida: {o}")));
        }
        q.add("orientation", o);
    }
    if let Some(c) = f.color {
        if c.len() != 6 || !c.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Usage(format!("color precisa de 6 dígitos hexadecimais (ex.: 0000ff): {c}")));
        }
        q.add("color", c);
    }
    if let Some(p) = f.people {
        q.add("people_count", word("people", p)?);
    }
    if let Some(a) = f.age {
        q.add("people_age", word("age", a)?);
    }
    if let Some(d) = f.date_from {
        q.add("date_from", word("date-from", d)?);
    }
    if let Some(d) = f.min_duration {
        q.add("min_duration", &d.to_string());
    }
    if let Some(d) = f.max_duration {
        q.add("max_duration", &d.to_string());
    }
    Ok(q.page(page))
}

/// Página de um vídeo ou foto. O site aceita qualquer texto antes do id e redireciona para o endereço canônico.
pub fn media_url(kind: Kind, id: u64) -> String {
    format!("{SITE}/{}/x-{id}/", if kind == Kind::Videos { "video" } else { "photo" })
}

/// Aceita `animals-h8stntd`, `@usuario` ou só o nome: letras, dígitos, `-`, `_` e `.`.
fn slug<'a>(name: &str, v: &'a str) -> Result<&'a str> {
    let v = v.trim_matches('/');
    if !v.is_empty() && v.len() <= 120 && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
        Ok(v)
    } else {
        Err(Error::Usage(format!("{name} inválido: {v}")))
    }
}

pub fn collection_url(collection: &str, page: u32) -> Result<String> {
    Ok(Query::new(format!("{SITE}/collections/{}/", slug("coleção", collection)?)).page(page))
}

pub fn user_url(user: &str, page: u32) -> Result<String> {
    Ok(Query::new(format!("{SITE}/@{}/", slug("usuário", user.trim_start_matches('@'))?)).page(page))
}

pub const POPULAR_VIDEOS: &str = "https://www.pexels.com/videos/";
pub const CURATED_PHOTOS: &str = "https://www.pexels.com/";
pub const DISCOVER: &str = "https://www.pexels.com/discover/";

/// Recorta o JSON de `__NEXT_DATA__` do HTML sem interpretar o HTML (busca de substring vetorizada).
pub fn next_data(html: &[u8]) -> Result<&[u8]> {
    let tag = memmem::find(html, b"id=\"__NEXT_DATA__\"").ok_or(Error::Layout("sem __NEXT_DATA__ (bloqueio ou site alterado)"))?;
    let start = tag + memchr::memchr(b'>', &html[tag..]).ok_or(Error::Layout("tag de script aberta"))? + 1;
    let len = memmem::find(&html[start..], b"</script>").ok_or(Error::Layout("script sem fechamento"))?;
    Ok(&html[start..start + len])
}

// Só os campos usados são declarados; o resto do JSON (centenas de chaves por item) é pulado pelo serde sem alocar.
// `Cow` + `borrow`: textos sem escape apontam direto para o buffer da página.
#[derive(Deserialize)]
struct Page<'a> {
    #[serde(borrow)]
    props: Props<'a>,
}
#[derive(Deserialize)]
struct Props<'a> {
    #[serde(borrow, rename = "pageProps")]
    page_props: PageProps<'a>,
}
/// Todas as páginas de mídia cabem aqui: cada tipo de página preenche um subconjunto.
#[derive(Deserialize)]
struct PageProps<'a> {
    /// busca, populares, curadoria, coleção
    #[serde(borrow, default, rename = "initialData")]
    initial_data: Option<List<'a>>,
    /// perfil de autor
    #[serde(borrow, default, rename = "firstPageOfMedia")]
    user_media: Option<List<'a>>,
    /// página de detalhe
    #[serde(borrow, default)]
    medium: Option<Item<'a>>,
    #[serde(borrow, default, rename = "firstPageOfSimilarMedia")]
    similar: Option<List<'a>>,
    #[serde(default, rename = "mediumStats")]
    stats: Option<Wrapped<Stats>>,
    #[serde(borrow, default, rename = "mediumDetails")]
    details: Option<Wrapped<&'a RawValue>>,
}
#[derive(Deserialize)]
struct Wrapped<T> {
    attributes: T,
}
#[derive(Deserialize, Clone, Copy)]
struct Stats {
    #[serde(default)]
    views: Option<u64>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(default)]
    likes: Option<u64>,
}
#[derive(Deserialize)]
struct List<'a> {
    #[serde(borrow, default)]
    data: Vec<Item<'a>>,
    #[serde(default)]
    pagination: Option<Pagination>,
    #[serde(borrow, default)]
    meta: Option<Meta<'a>>,
}
#[derive(Deserialize)]
struct Meta<'a> {
    #[serde(borrow, default)]
    related_searches: Vec<Related<'a>>,
}
/// Busca relacionada sugerida pelo site, com o número de resultados que ela tem.
#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Related<'a> {
    #[serde(borrow)]
    pub term: Cow<'a, str>,
    #[serde(default)]
    pub count: u64,
}
/// Paginação numerada (busca, coleção, autor, parecidos). Populares e curadoria usam cursor e só expõem a primeira página.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct Pagination {
    #[serde(default)]
    pub current_page: u32,
    #[serde(default)]
    pub total_pages: u32,
    #[serde(default)]
    pub total_results: u64,
}
#[derive(Deserialize)]
struct Item<'a> {
    #[serde(borrow, rename = "type")]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    attributes: Attr<'a>,
}
#[derive(Deserialize)]
struct Attr<'a> {
    id: u64,
    #[serde(borrow, default)]
    slug: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    title: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    description: Option<Cow<'a, str>>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(borrow, default)]
    license: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    created_at: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    user: Option<User<'a>>,
    #[serde(borrow, default)]
    tags: Vec<Tag<'a>>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    fps: Option<f32>,
    #[serde(borrow, default)]
    video: Option<Video<'a>>,
    #[serde(borrow, default)]
    image: Option<Image<'a>>,
}
#[derive(Deserialize)]
struct User<'a> {
    #[serde(borrow, default)]
    first_name: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    last_name: Option<Cow<'a, str>>,
}
/// Nas buscas a tag é um objeto `{name, search_term}`; em coleções, autores e detalhe é só o texto.
#[derive(Deserialize)]
#[serde(untagged)]
enum Tag<'a> {
    Named {
        #[serde(borrow)]
        name: Cow<'a, str>,
    },
    Plain(#[serde(borrow)] Cow<'a, str>),
}

impl<'a> Tag<'a> {
    fn into_name(self) -> Cow<'a, str> {
        match self {
            Tag::Named { name } | Tag::Plain(name) => name,
        }
    }
}
#[derive(Deserialize)]
struct Video<'a> {
    #[serde(borrow, default)]
    video_files: Vec<VideoFile<'a>>,
}
#[derive(Deserialize)]
struct VideoFile<'a> {
    #[serde(borrow, default)]
    file_type: Option<Cow<'a, str>>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    fps: Option<f32>,
    #[serde(borrow)]
    link: Cow<'a, str>,
}
#[derive(Deserialize)]
struct Image<'a> {
    #[serde(borrow, default)]
    download_link: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    large: Option<Cow<'a, str>>,
}

/// Como transformar um item em linha.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub pick: Pick,
    pub photo_width: u32,
    /// Incluir todas as rendições mp4 do vídeo em `files`.
    pub all_files: bool,
}

/// Vídeo sem arquivo que sirva a `pick`, foto sem link e tipos desconhecidos devolvem `None`.
fn to_row<'a>(item: Item<'a>, shape: &Shape) -> Option<Row<'a>> {
    let a = item.attributes;
    let (kind, url, width, height, fps, files) = match item.kind.as_ref() {
        "video" => {
            let mut files = a.video?.video_files;
            files.retain(|f| f.file_type.as_deref() == Some("video/mp4"));
            let i = shape.pick.best(files.iter().map(|f| (f.width.unwrap_or(0), f.height.unwrap_or(0))))?;
            let (url, w, h, fps) = (files[i].link.clone(), files[i].width.unwrap_or(0), files[i].height.unwrap_or(0), files[i].fps.or(a.fps));
            let all = if shape.all_files {
                files.into_iter().map(|f| FileRow { url: f.link, width: f.width.unwrap_or(0), height: f.height.unwrap_or(0), fps: f.fps }).collect()
            } else {
                Vec::new()
            };
            ("video", url, w, h, fps, all)
        }
        "photo" => {
            let src = a.image.and_then(|i| i.download_link.or(i.large))?;
            // o original pode ter 50 MP; o CDN redimensiona pelo parâmetro `w`
            let base = src.split('?').next().unwrap_or(&src);
            let (w, h) = (a.width.unwrap_or(0), a.height.unwrap_or(0));
            let out_w = if w == 0 { shape.photo_width } else { w.min(shape.photo_width) };
            let out_h = if w == 0 { 0 } else { (u64::from(h) * u64::from(out_w) / u64::from(w)) as u32 };
            ("photo", Cow::Owned(format!("{base}?auto=compress&cs=tinysrgb&w={out_w}")), out_w, out_h, None, Vec::new())
        }
        _ => return None,
    };
    let author = a.user.map(|u| [u.first_name.as_deref().unwrap_or(""), u.last_name.as_deref().unwrap_or("")].join(" ").trim().to_owned()).unwrap_or_default();
    let slug = a.slug.as_deref().unwrap_or("");
    Some(Row {
        id: a.id,
        kind,
        page: if slug.is_empty() { format!("{SITE}/{kind}/{}/", a.id) } else { format!("{SITE}/{kind}/{slug}-{}/", a.id) },
        url,
        width,
        height,
        duration: a.duration,
        fps,
        title: a.title.unwrap_or_default(),
        description: a.description.unwrap_or_default(),
        tags: a.tags.into_iter().map(Tag::into_name).collect(),
        author,
        license: a.license.unwrap_or(Cow::Borrowed("Pexels")),
        created_at: a.created_at,
        files,
        ..Row::default()
    })
}

/// Resultado de uma página de lista.
pub struct Listing<'a> {
    pub rows: Vec<Row<'a>>,
    /// Itens que a página trouxe, antes de descartar os sem arquivo utilizável.
    pub items: usize,
    pub pagination: Option<Pagination>,
    pub related: Vec<Related<'a>>,
}

fn props(html: &[u8]) -> Result<PageProps<'_>> {
    Ok(serde_json::from_slice::<Page<'_>>(next_data(html)?)?.props.page_props)
}

fn listing<'a>(list: List<'a>, shape: &Shape) -> Listing<'a> {
    let items = list.data.len();
    Listing { rows: list.data.into_iter().filter_map(|i| to_row(i, shape)).collect(), items, pagination: list.pagination,
              related: list.meta.map(|m| m.related_searches).unwrap_or_default() }
}

/// Busca, populares, curadoria, coleção ou perfil de autor: qualquer página que liste mídia.
pub fn parse_list<'a>(html: &'a [u8], shape: &Shape) -> Result<Listing<'a>> {
    let p = props(html)?;
    let list = p.initial_data.or(p.user_media).ok_or(Error::Layout("página sem lista de mídia (consulta recusada ou endereço errado?)"))?;
    Ok(listing(list, shape))
}

/// Página de detalhe: a mídia com estatísticas e detalhes, e a primeira página de mídia parecida.
pub fn parse_detail<'a>(html: &'a [u8], shape: &Shape) -> Result<(Option<Row<'a>>, Listing<'a>)> {
    let p = props(html)?;
    let medium = p.medium.ok_or(Error::Layout("página sem a mídia (id inexistente?)"))?;
    let mut row = to_row(medium, shape);
    if let Some(r) = row.as_mut() {
        if let Some(Wrapped { attributes: s }) = p.stats {
            (r.views, r.downloads, r.likes) = (s.views, s.downloads, s.likes);
        }
        r.details = p.details.map(|d| d.attributes);
    }
    let similar = p.similar.map(|l| listing(l, shape)).unwrap_or(Listing { rows: Vec::new(), items: 0, pagination: None, related: Vec::new() });
    Ok((row, similar))
}

/// Uma coleção da página Descobrir.
#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Collection<'a> {
    #[serde(borrow)]
    pub slug: Cow<'a, str>,
    #[serde(borrow, default)]
    pub title: Cow<'a, str>,
    #[serde(borrow, default)]
    pub description: Cow<'a, str>,
    #[serde(default)]
    pub photos_count: u32,
    #[serde(default)]
    pub videos_count: u32,
    #[serde(default, rename(deserialize = "collection_media_count"))]
    pub total: u32,
}

#[derive(Deserialize)]
struct DiscoverPage<'a> {
    #[serde(borrow)]
    props: DiscoverProps<'a>,
}
#[derive(Deserialize)]
struct DiscoverProps<'a> {
    #[serde(borrow, rename = "pageProps")]
    page_props: DiscoverPageProps<'a>,
}
#[derive(Deserialize)]
struct DiscoverPageProps<'a> {
    #[serde(borrow, rename = "initialData")]
    initial_data: Discover<'a>,
}
#[derive(Deserialize)]
struct Discover<'a> {
    #[serde(borrow, default, rename = "curatedCollections")]
    curated: Option<Curated<'a>>,
    /// grupos temáticos: uma lista de coleções por tema
    #[serde(borrow, default)]
    collections: Vec<Vec<Wrapped<Collection<'a>>>>,
}
#[derive(Deserialize)]
struct Curated<'a> {
    #[serde(borrow, default)]
    data: Vec<Wrapped<Collection<'a>>>,
}

/// Coleções em destaque (curadoria primeiro, depois os grupos temáticos), sem repetição.
pub fn parse_collections(html: &[u8]) -> Result<Vec<Collection<'_>>> {
    let d = serde_json::from_slice::<DiscoverPage<'_>>(next_data(html)?)?.props.page_props.initial_data;
    let mut out: Vec<Collection<'_>> = Vec::new();
    for c in d.curated.map(|c| c.data).unwrap_or_default().into_iter().chain(d.collections.into_iter().flatten()) {
        if !out.iter().any(|o| o.slug == c.attributes.slug) {
            out.push(c.attributes);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEOS: &[u8] = include_bytes!("../tests/videos.html");
    const PHOTOS: &[u8] = include_bytes!("../tests/photos.html");

    fn shape(orientation: Option<&str>) -> Shape {
        Shape { pick: Pick::for_orientation(orientation), photo_width: 2160, all_files: false }
    }

    #[test]
    fn search_url_encodes_validates_and_orders_filters() {
        let f = Server { orientation: Some("portrait"), ..Server::default() };
        assert_eq!(search_url(Kind::Videos, " bull shark & río ", 0, &f).unwrap(), "https://www.pexels.com/search/videos/bull%20shark%20%26%20r%C3%ADo/?orientation=portrait");
        assert_eq!(search_url(Kind::Photos, "a/b?c#d", 3, &Server::default()).unwrap(), "https://www.pexels.com/search/a%2Fb%3Fc%23d/?page=3");
        assert_eq!(search_url(Kind::Photos, "ice", 1, &Server::default()).unwrap(), "https://www.pexels.com/search/ice/");
        let all = Server { orientation: Some("landscape"), color: Some("0000ff"), people: Some("1"), age: Some("adult"), date_from: Some("last_year"), min_duration: Some(5), max_duration: Some(15) };
        assert_eq!(search_url(Kind::Videos, "ice", 2, &all).unwrap(),
                   "https://www.pexels.com/search/videos/ice/?orientation=landscape&color=0000ff&people_count=1&people_age=adult&date_from=last_year&min_duration=5&max_duration=15&page=2");
        for bad in [Server { orientation: Some("portrait&evil=1"), ..Server::default() }, Server { color: Some("blue"), ..Server::default() },
                    Server { people: Some("1&x=2"), ..Server::default() }, Server { date_from: Some("a b"), ..Server::default() }] {
            assert!(search_url(Kind::Videos, "x", 1, &bad).is_err(), "{bad:?}");
        }
        assert!(search_url(Kind::Videos, "   ", 1, &Server::default()).is_err());
    }

    #[test]
    fn other_urls_reject_path_tricks() {
        assert_eq!(media_url(Kind::Videos, 34956040), "https://www.pexels.com/video/x-34956040/");
        assert_eq!(collection_url("/animals-h8stntd/", 2).unwrap(), "https://www.pexels.com/collections/animals-h8stntd/?page=2");
        assert_eq!(user_url("@arthousestudio", 1).unwrap(), "https://www.pexels.com/@arthousestudio/");
        assert!(collection_url("../admin", 1).is_err() && collection_url("a/b", 1).is_err() && user_url("a?b=1", 1).is_err() && user_url("", 1).is_err());
    }

    #[test]
    fn next_data_is_cut_out_or_reported() {
        assert_eq!(next_data(br#"<p>a</p><script id="__NEXT_DATA__" type="application/json" crossorigin="anonymous">{"a":1}</script><p>"#).unwrap(), br#"{"a":1}"#);
        assert!(matches!(next_data(b"<html>Just a moment...</html>"), Err(Error::Layout(_))));
        assert!(matches!(next_data(br#"<script id="__NEXT_DATA__">{"a":1}"#), Err(Error::Layout(_))));
    }

    #[test]
    fn parses_real_video_search_page() {
        let l = parse_list(VIDEOS, &shape(Some("portrait"))).unwrap();
        assert_eq!((l.rows.len(), l.items), (2, 2));
        let r = &l.rows[0];
        assert_eq!((r.id, r.kind, r.width, r.height), (39056886, "video", 1080, 1920));
        assert_eq!(r.url, "https://videos.pexels.com/video-files/39056886/16620047_1080_1920_30fps.mp4");
        assert_eq!((r.title.as_ref(), r.author.as_str(), r.duration, r.fps), ("Majestic Shark Swimming in Aquarium", "Tommy Shih", Some(14.0), Some(30.0)));
        assert!(r.tags.iter().any(|t| t == "Aquarium") && r.page.ends_with("-39056886/") && r.files.is_empty() && r.created_at.is_some());
        assert_eq!(l.pagination, Some(Pagination { current_page: 2, total_pages: 102, total_results: 2429 }));
        // alvo em paisagem: nenhum arquivo desses vídeos em retrato serve, mas a página tinha 2 itens
        let none = parse_list(VIDEOS, &shape(Some("landscape"))).unwrap();
        assert_eq!((none.rows.len(), none.items), (0, 2));
        let all = parse_list(VIDEOS, &Shape { all_files: true, ..shape(Some("portrait")) }).unwrap();
        assert_eq!(all.rows[0].files.len(), 4);
    }

    #[test]
    fn parses_real_photo_search_page_with_related_searches() {
        let l = parse_list(PHOTOS, &shape(None)).unwrap();
        assert_eq!(l.rows[0].url, "https://images.pexels.com/photos/5022610/pexels-photo-5022610.jpeg?auto=compress&cs=tinysrgb&w=2160");
        assert_eq!((l.rows[0].kind, l.rows[0].width, l.rows[0].height, l.rows[0].duration), ("photo", 2160, 2880, None));
        assert_eq!(l.rows[0].author, "ArtHouse Studio");
        let rel = parse_list(include_bytes!("../tests/related.html"), &shape(None)).unwrap().related;
        assert_eq!(rel, vec![Related { term: "glaciers".into(), count: 12764 }, Related { term: "iceberg".into(), count: 4826 }]);
    }

    #[test]
    fn parses_real_detail_pages_with_stats_details_and_similar() {
        let (row, similar) = parse_detail(include_bytes!("../tests/video_detail.html"), &Shape { all_files: true, ..shape(None) }).unwrap();
        let r = row.unwrap();
        assert_eq!((r.id, r.title.as_ref(), r.views, r.downloads, r.likes), (34956040, "Breathtaking Drone View of a Frozen Mountain Landscape", Some(258225), Some(949), Some(13)));
        assert_eq!((r.files.len(), r.width, r.height), (6, 1920, 1080), "sem orientação pedida, um vídeo em paisagem sai em 1080p");
        assert!(r.details.unwrap().get().contains("59.94"));
        assert_eq!(similar.rows.iter().map(|s| s.id).collect::<Vec<_>>(), [34956041, 34956049]);
        assert_eq!(similar.pagination.unwrap().total_results, 1031);

        let (row, _) = parse_detail(include_bytes!("../tests/photo_detail.html"), &shape(None)).unwrap();
        let r = row.unwrap();
        assert_eq!((r.id, r.kind, r.views, r.downloads, r.likes), (5022610, "photo", Some(295929), Some(1012), Some(21)));
        assert!(r.details.unwrap().get().contains("GFX 50S"), "EXIF repassado");
    }

    #[test]
    fn parses_real_collection_user_and_popular_pages() {
        let c = parse_list(include_bytes!("../tests/collection.html"), &shape(None)).unwrap();
        assert_eq!(c.rows.iter().map(|r| (r.kind, r.id)).collect::<Vec<_>>(), [("photo", 11760851), ("photo", 12715260)]);
        assert_eq!(c.pagination.unwrap().total_results, 166);
        let u = parse_list(include_bytes!("../tests/user.html"), &shape(None)).unwrap();
        assert_eq!(u.rows.iter().map(|r| (r.kind, r.id)).collect::<Vec<_>>(), [("video", 8515873), ("video", 8515872)]);
        assert_eq!(u.pagination, Some(Pagination { current_page: 2, total_pages: 823, total_results: 9869 }));
        let p = parse_list(include_bytes!("../tests/popular.html"), &shape(None)).unwrap();
        assert_eq!(p.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [39527910, 36550248]);
        assert_eq!(p.pagination, Some(Pagination::default()), "populares usam cursor: sem páginas numeradas");
    }

    #[test]
    fn parses_real_discover_page_without_duplicates() {
        let cs = parse_collections(include_bytes!("../tests/discover.html")).unwrap();
        assert_eq!(cs[0].slug, "behind-the-camera-r7ttfbd");
        assert_eq!((cs[0].title.as_ref(), cs[0].photos_count, cs[0].videos_count, cs[0].total), ("Behind The Camera", 71, 0, 71));
        assert!(cs.len() >= 4 && cs.iter().any(|c| c.slug == "stay-healthy-jmm4dr7"));
        let mut slugs: Vec<_> = cs.iter().map(|c| c.slug.as_ref()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), cs.len());
    }

    #[test]
    fn rejects_pages_without_the_expected_block() {
        let empty = br#"<script id="__NEXT_DATA__">{"props":{"pageProps":{}}}</script>"#;
        assert!(matches!(parse_list(empty, &shape(None)), Err(Error::Layout(_))));
        assert!(matches!(parse_detail(empty, &shape(None)), Err(Error::Layout(_))));
        assert!(matches!(parse_list(br#"<script id="__NEXT_DATA__">{"props":</script>"#, &shape(None)), Err(Error::Json(_))));
        assert!(parse_collections(empty).is_err());
    }
}
