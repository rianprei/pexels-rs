# pexels-rs

Vídeos e fotos da Pexels pela linha de comando, sem chave de API: lê as páginas públicas do site (o JSON embutido em `__NEXT_DATA__`). Com `--api` e `PEXELS_API_KEY` usa a API oficial. Feito em 17/11/2026 porque a emissão de chaves está pausada desde 06/11.

```bash
cargo build -q --release          # binário em target/release/pexels-rs
cargo test -q --release           # 18 testes; os de leitura usam páginas reais reduzidas em tests/
target/release/pexels-rs help
```

## Comparação com a API oficial
| Recurso | API oficial | pexels-rs (sem chave) |
|---|---|---|
| Buscar fotos e vídeos, orientação, paginação | sim | sim (`videos`, `photos`, `--limit` pagina sozinho) |
| Cor da foto | nomes ou hex | hex (`--color`) |
| Duração mín./máx. do vídeo | sim | sim (`--min-duration`, `--max-duration`) |
| Vídeos populares / fotos da curadoria | paginado | só a primeira página (18 / 12); paginado com `--api` |
| Mídia por id | sim | sim (`get`), com todas as rendições |
| Coleções em destaque | sim | sim (`collections`, 92 em 17/11) |
| Mídia de uma coleção | paginado | só os 12 primeiros |
| Título, descrição e tags dos vídeos | não | sim |
| Visualizações, downloads, curtidas | não | sim (`get`) |
| EXIF da foto (câmera, abertura, ISO, data) | não | sim (`get`, campo `details`) |
| Mídia parecida | não | sim (`similar`, 12) |
| Buscas relacionadas com contagem | não | sim (`related`) |
| Mídia de um autor | não | sim (`user`, paginado) |
| Filtro por pessoas, idade, data de envio | não | sim (`--people`, `--age`, `--date-from`) |
| Filtro local por palavra, fps, largura, tipo, duração | não | sim (`--with`, `--without`, `--min-fps`, `--min-width`, `--only`, `--shorter-than`, `--longer-than`) |
| Limite de requisições | 200/h, 20.000/mês | sem cota declarada; uma página por vez, 0,7 s de pausa, cache de 6 h |
| Estabilidade | contrato documentado | quebra se o site mudar (erro "página em formato inesperado") |

## Limites conhecidos
- Coleções, populares e curadoria: o HTML público só traz a primeira página.
- Buscas em rajada paralela recebem 403; por isso é sequencial. Um 403/429/5xx é repetido após 3 s e 8 s.
- Os termos de uso da Pexels não preveem acesso automatizado ao site.
- Download só de `videos.pexels.com` e `images.pexels.com`.

## Qualidade
Revisado com a skill `rust-review` de [morganlinton/BeautifulRust](https://github.com/morganlinton/BeautifulRust) (MIT). O `Cargo.toml` adota o conjunto de lints dela na variante para aplicação: `clippy::pedantic`, sem `unwrap` fora de teste, sem conversão `as`, sem parâmetro booleano, sem `unsafe` (proibido). O erro implementa `std::error::Error` e preserva a causa (`source`).

