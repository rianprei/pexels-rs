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
            Error::Usage(m) => write!(f, "{m}"),
            Error::Status(code, what) => write!(f, "{what}: HTTP {code}"),
            Error::Curl(e) => write!(f, "falha de rede: {e}"),
            Error::Layout(m) => write!(f, "página em formato inesperado: {m}"),
            Error::Json(e) => write!(f, "JSON inválido: {e}"),
            Error::Io(path, e) => write!(f, "{path}: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            Error::Curl(e) => Some(e),
            Error::Io(_, e) => Some(e),
            Error::Usage(_) | Error::Status(..) | Error::Layout(_) => None,
        }
    }
}

impl From<curl::Error> for Error {
    fn from(e: curl::Error) -> Self {
        Error::Curl(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
