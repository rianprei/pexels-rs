//! Linha de saída comum a todas as fontes, escolha do arquivo de vídeo e filtros aplicados no cliente.
use serde::Serialize;
use serde_json::value::RawValue;
use std::borrow::Cow;

/// Uma rendição de um vídeo (só com `--files`).
#[derive(Debug, Serialize, PartialEq)]
pub struct FileRow<'a> {
    pub url: Cow<'a, str>,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
}

/// Um vídeo ou foto, reduzido ao que serve a quem consome. Uma linha JSON por resultado.
/// A API oficial não entrega `title`, `description`, `tags`, `views`, `downloads`, `likes` nem `details` para vídeos.
#[derive(Debug, Serialize, Default)]
pub struct Row<'a> {
    pub id: u64,
    pub kind: &'static str,
    pub page: String,
    pub url: Cow<'a, str>,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    pub title: Cow<'a, str>,
    pub description: Cow<'a, str>,
    pub tags: Vec<Cow<'a, str>>,
    pub author: String,
    pub license: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileRow<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    /// Bloco de detalhes da página (EXIF da foto: câmera, abertura, ISO, data; fps/duração do vídeo), repassado sem reinterpretar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<&'a RawValue>,
}

/// Preferências para escolher o arquivo de um vídeo.
#[derive(Debug, Clone, Copy)]
pub struct Pick {
    pub min_height: u32,
    /// Lado menor preferido. 1080 serve aos dois formatos de saída: 1080x1920 em retrato e 1920x1080 em paisagem.
    pub target_short: u32,
    /// `Some(true)` só retrato, `Some(false)` só paisagem, `None` qualquer um.
    pub portrait: Option<bool>,
}

impl Pick {
    pub fn for_orientation(orientation: Option<&str>) -> Self {
        match orientation {
            Some("portrait") => Pick { min_height: 1280, target_short: 1080, portrait: Some(true) },
            Some("landscape") => Pick { min_height: 1080, target_short: 1080, portrait: Some(false) },
            _ => Pick { min_height: 720, target_short: 1080, portrait: None },
        }
    }

    /// Índice do melhor candidato `(largura, altura)`: respeita orientação e altura mínima, com o lado menor mais próximo do alvo
    /// (empate: o maior). 4K só custa banda e disco para quem renderiza em 1080.
    pub fn best(&self, sizes: impl Iterator<Item = (u32, u32)>) -> Option<usize> {
        sizes
            .enumerate()
            .filter(|&(_, (w, h))| h >= self.min_height && self.portrait.is_none_or(|p| (h > w) == p))
            .min_by_key(|&(_, (w, h))| (w.min(h).abs_diff(self.target_short), u32::MAX - h))
            .map(|(i, _)| i)
    }
}

/// Filtros que o site não oferece na URL: aplicados aqui, sobre os dados que a página já trouxe.
#[derive(Debug, Default)]
pub struct Filter {
    pub only: Option<&'static str>,
    pub min_width: u32,
    pub min_fps: f32,
    pub min_duration: f32,
    pub max_duration: f32,
    /// Em minúsculas. Todas precisam aparecer em título, descrição ou tags.
    pub with: Vec<String>,
    /// Em minúsculas. Nenhuma pode aparecer.
    pub without: Vec<String>,
}

impl Filter {
    pub fn accepts(&self, r: &Row<'_>) -> bool {
        if self.only.is_some_and(|k| k != r.kind) || r.width < self.min_width {
            return false;
        }
        if r.kind == "video" {
            let (fps, dur) = (r.fps.unwrap_or(0.0), r.duration.unwrap_or(0.0));
            if fps < self.min_fps || dur < self.min_duration || (self.max_duration > 0.0 && dur > self.max_duration) {
                return false;
            }
        }
        if self.with.is_empty() && self.without.is_empty() {
            return true;
        }
        let mut text = String::with_capacity(r.title.len() + r.description.len() + 16 * r.tags.len());
        for part in [r.title.as_ref(), r.description.as_ref()].into_iter().chain(r.tags.iter().map(|t| t.as_ref())) {
            text.push_str(part);
            text.push('\n');
        }
        text.make_ascii_lowercase();
        self.with.iter().all(|w| text.contains(w.as_str())) && !self.without.iter().any(|w| text.contains(w.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &'static str, fps: f32, dur: f32, title: &'static str, tags: &[&'static str]) -> Row<'static> {
        Row { kind, width: 1080, height: 1920, fps: Some(fps), duration: Some(dur), title: Cow::Borrowed(title),
              tags: tags.iter().map(|t| Cow::Borrowed(*t)).collect(), ..Row::default() }
    }

    #[test]
    fn pick_prefers_height_closest_to_target() {
        let p = Pick::for_orientation(Some("portrait"));
        assert_eq!(p.best([(360, 640), (1080, 1920), (2160, 3840)].into_iter()), Some(1));
        assert_eq!(p.best([(1920, 1080), (3840, 2160)].into_iter()), None, "paisagem não serve a retrato");
        assert_eq!(Pick::for_orientation(None).best([(1280, 720), (1920, 1080), (3840, 2160)].into_iter()), Some(1), "paisagem: 1080p, não 4K");
        assert_eq!(Pick::for_orientation(None).best([(720, 1280), (1080, 1920), (2160, 3840)].into_iter()), Some(1), "retrato: 1080x1920");
        assert_eq!(Pick { target_short: 2160, ..Pick::for_orientation(None) }.best([(1920, 1080), (3840, 2160)].into_iter()), Some(1));
    }

    #[test]
    fn filter_checks_kind_numbers_and_words() {
        let shark = row("video", 30.0, 14.0, "Sharks in Aquarium", &["Aquarium", "Blue Water"]);
        assert!(Filter::default().accepts(&shark));
        assert!(!Filter { without: vec!["aquarium".into()], ..Filter::default() }.accepts(&shark));
        assert!(Filter { with: vec!["shark".into(), "blue water".into()], ..Filter::default() }.accepts(&shark));
        assert!(!Filter { with: vec!["reef".into()], ..Filter::default() }.accepts(&shark));
        assert!(!Filter { min_fps: 50.0, ..Filter::default() }.accepts(&shark));
        assert!(!Filter { max_duration: 10.0, ..Filter::default() }.accepts(&shark));
        assert!(!Filter { min_duration: 20.0, ..Filter::default() }.accepts(&shark));
        assert!(!Filter { only: Some("photo"), ..Filter::default() }.accepts(&shark));
        assert!(!Filter { min_width: 2000, ..Filter::default() }.accepts(&shark));
        // fps e duração não se aplicam a foto
        assert!(Filter { min_fps: 50.0, min_duration: 5.0, ..Filter::default() }.accepts(&row("photo", 0.0, 0.0, "x", &[])));
    }
}
