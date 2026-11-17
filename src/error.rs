use std::fmt;

/// Tudo que pode dar errado, com mensagem pronta para o operador.
#[derive(Debug)]
pub(crate) enum Error {
    Usage(String),
    /// Resposta HTTP com status de erro.
    Status(u32, &'static str),
    /// Erro da libcurl (rede, TLS, tempo esgotado), com a causa preservada.
    Curl(curl::Error),
    /// A página veio, mas não tem o formato esperado (a Pexels mudou o site ou bloqueou a requisição).
    Layout(&'static str),
    Json(serde_json::Error),
    Io(String, std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(m) => write!(f, "{m}"),
            Self::Status(code, what) => write!(f, "{what}: HTTP {code}"),
            Self::Curl(e) => write!(f, "falha de rede: {e}"),
            Self::Layout(m) => write!(f, "página em formato inesperado: {m}"),
            Self::Json(e) => write!(f, "JSON inválido: {e}"),
            Self::Io(path, e) => write!(f, "{path}: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Curl(e) => Some(e),
            Self::Io(_, e) => Some(e),
            Self::Usage(_) | Self::Status(..) | Self::Layout(_) => None,
        }
    }
}

impl From<curl::Error> for Error {
    fn from(e: curl::Error) -> Self {
        Self::Curl(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
