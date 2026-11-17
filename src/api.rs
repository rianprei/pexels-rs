//! Fonte com chave: API oficial (porta dos clientes Python salvoventura/pypexels e AguilarLagunasArturo/pexels-api, MIT).
use crate::error::{Error, Result};
use crate::model::{scaled_height, Pick, Row};
use crate::web::{encode_into, Kind};
use serde::Deserialize;
use std::borrow::Cow;
use std::fmt::Write as _;

const API: &str = "https://api.pexels.com";

/// Caminhos da API oficial sem consulta: vídeos populares e fotos da curadoria.
pub(crate) fn feed_url(kind: Kind, page: u32, per_page: u32) -> String {
    format!(
        "{API}{}?per_page={}&page={}",
        if kind == Kind::Videos {
            "/videos/popular"
        } else {
            "/v1/curated"
        },
        per_page.clamp(1, 80),
        page.max(1)
    )
}

pub(crate) fn search_url(
    kind: Kind,
    query: &str,
    page: u32,
    per_page: u32,
    orientation: Option<&str>,
) -> Result<String> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::Usage("consulta vazia".into()));
    }
    let mut url = format!(
        "{API}{}?query=",
        if kind == Kind::Videos {
            "/videos/search"
        } else {
            "/v1/search"
        }
    );
    encode_into(&mut url, query);
    let _ = write!(
        url,
        "&per_page={}&page={}",
        per_page.clamp(1, 80),
        page.max(1)
    ); // escrever em String não falha
    if let Some(o) = orientation {
        if !matches!(o, "portrait" | "landscape" | "square") {
            return Err(Error::Usage(format!("orientation inválida: {o}")));
        }
        url.push_str("&orientation=");
        url.push_str(o);
    }
    Ok(url)
}

#[derive(Deserialize)]
struct Body<'a> {
    #[serde(borrow, default)]
    videos: Vec<ApiVideo<'a>>,
    #[serde(borrow, default)]
    photos: Vec<ApiPhoto<'a>>,
}
#[derive(Deserialize)]
struct ApiVideo<'a> {
    id: u64,
    #[serde(borrow, default)]
    url: Option<Cow<'a, str>>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(borrow, default)]
    user: Option<Named<'a>>,
    #[serde(borrow, default)]
    video_files: Vec<ApiFile<'a>>,
}
#[derive(Deserialize)]
struct Named<'a> {
    #[serde(borrow, default)]
    name: Option<Cow<'a, str>>,
}
#[derive(Deserialize)]
struct ApiFile<'a> {
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
struct ApiPhoto<'a> {
    id: u64,
    #[serde(borrow, default)]
    url: Option<Cow<'a, str>>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(borrow, default)]
    alt: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    photographer: Option<Cow<'a, str>>,
    #[serde(borrow)]
    src: Src<'a>,
}
#[derive(Deserialize)]
struct Src<'a> {
    #[serde(borrow)]
    original: Cow<'a, str>,
}

pub(crate) fn parse<'a>(
    json: &'a [u8],
    kind: Kind,
    pick: Pick,
    photo_width: u32,
) -> Result<Vec<Row<'a>>> {
    let body: Body<'a> = serde_json::from_slice(json)?;
    let page = |u: Option<Cow<'a, str>>| u.map(Cow::into_owned).unwrap_or_default();
    Ok(match kind {
        Kind::Videos => body
            .videos
            .into_iter()
            .filter_map(|v| {
                let mut files = v.video_files;
                files.retain(|f| f.file_type.as_deref() == Some("video/mp4"));
                let f = files.swap_remove(
                    pick.best(
                        files
                            .iter()
                            .map(|f| (f.width.unwrap_or(0), f.height.unwrap_or(0))),
                    )?,
                );
                Some(Row {
                    id: v.id,
                    kind: "video",
                    page: page(v.url),
                    url: f.link,
                    width: f.width.unwrap_or(0),
                    height: f.height.unwrap_or(0),
                    duration: v.duration,
                    fps: f.fps,
                    author: v
                        .user
                        .and_then(|u| u.name)
                        .map(Cow::into_owned)
                        .unwrap_or_default(),
                    license: Cow::Borrowed("Pexels"),
                    ..Row::default()
                })
            })
            .collect(),
        Kind::Photos => body
            .photos
            .into_iter()
            .map(|p| {
                let base = p.src.original.split('?').next().unwrap_or("").to_owned();
                let w = p.width.unwrap_or(photo_width).min(photo_width);
                let h = scaled_height(p.width.unwrap_or(0), p.height.unwrap_or(0), w);
                Row {
                    id: p.id,
                    kind: "photo",
                    page: page(p.url),
                    url: Cow::Owned(format!("{base}?auto=compress&cs=tinysrgb&w={w}")),
                    width: w,
                    height: h,
                    title: p.alt.unwrap_or_default(),
                    author: p.photographer.map(Cow::into_owned).unwrap_or_default(),
                    license: Cow::Borrowed("Pexels"),
                    ..Row::default()
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matches_official_api() {
        assert_eq!(search_url(Kind::Videos, "bull shark", 0, 500, Some("portrait")).unwrap(),
                   "https://api.pexels.com/videos/search?query=bull%20shark&per_page=80&page=1&orientation=portrait");
        assert!(search_url(Kind::Photos, "x", 2, 5, None)
            .unwrap()
            .starts_with("https://api.pexels.com/v1/search?query=x&per_page=5&page=2"));
        assert!(search_url(Kind::Photos, "x", 1, 5, Some("x&y")).is_err());
    }

    #[test]
    fn picks_file_closest_to_target_and_skips_unusable() {
        let json = br#"{"videos":[{"id":7,"url":"https://www.pexels.com/video/7/","duration":12,"user":{"name":"Ana"},"video_files":[
            {"file_type":"video/mp4","height":3840,"width":2160,"link":"https://videos.pexels.com/4k.mp4"},
            {"file_type":"video/mp4","height":1920,"width":1080,"fps":30,"link":"https://videos.pexels.com/hd.mp4"},
            {"file_type":"application/x-mpegURL","height":1920,"width":1080,"link":"https://videos.pexels.com/hls.m3u8"}]},
            {"id":8,"video_files":[{"file_type":"video/mp4","height":640,"width":360,"link":"https://videos.pexels.com/low.mp4"}]}]}"#;
        let rows = parse(
            json,
            Kind::Videos,
            Pick::for_orientation(Some("portrait")),
            2160,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].url.as_ref(), rows[0].author.as_str(), rows[0].fps),
            ("https://videos.pexels.com/hd.mp4", "Ana", Some(30.0))
        );
        assert!(parse(
            br#"{"error":"x"}"#,
            Kind::Videos,
            Pick::for_orientation(None),
            2160
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn photo_is_resized_by_cdn_parameter() {
        let json = br#"{"photos":[{"id":3,"url":"p","width":4000,"height":6000,"alt":"a glacier","photographer":"Bo","src":{"original":"https://images.pexels.com/photos/3/x.jpeg"}}]}"#;
        let r = &parse(json, Kind::Photos, Pick::for_orientation(None), 2000).unwrap()[0];
        assert_eq!(
            (r.url.as_ref(), r.width, r.height, r.title.as_ref()),
            (
                "https://images.pexels.com/photos/3/x.jpeg?auto=compress&cs=tinysrgb&w=2000",
                2000,
                3000,
                "a glacier"
            )
        );
    }
}
