use std::fmt;

/// Tudo que pode dar errado, com mensagem pronta para o operador.
#[derive(Debug)]
pub enum Error {
    Usage(String),
    /// Resposta HTTP com status de erro.
    Status(u16, &'static str),
    Transport(String),
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
            Error::Transport(m) => write!(f, "falha de rede: {m}"),
            Error::Layout(m) => write!(f, "página em formato inesperado: {m}"),
            Error::Json(e) => write!(f, "JSON inválido: {e}"),
            Error::Io(path, e) => write!(f, "{path}: {e}"),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
