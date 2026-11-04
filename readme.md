# polymark.rs

Переводит текст между обычным текстом, Markdown, HTML, форматированным текстом из буфера обмена и Telegram MarkdownV2. Все форматы проходят через одну модель документа, поэтому любая пара форматов работает одинаково, а то, что целевой формат выразить не может, попадает в предупреждения, а не пропадает молча.

## Установка

Rust:

```bash
cargo add polymark-core
```

JavaScript:

```bash
pnpm add @meritt/polymark-wasm
```

Swift:

```swift
.package(url: "https://github.com/meritt/polymark.rs", exact: "0.1.0")
```

## Использование

Rust:

```rust
use polymark_core::{Input, Output, transform};

let converted = transform("**Жирный**", Input::Markdown, Output::Html)?; // result: "<p><strong>Жирный</strong></p>"
```

JavaScript:

```js
import { load } from '@meritt/polymark-wasm';

const polymark = await load();
polymark.transform('**Жирный**', 'markdown', 'html');
// { result: '<p><strong>Жирный</strong></p>', warnings: {} }
polymark.transform('<table><tr><td>a</td></tr></table>', 'html', 'telegram');
// { result: '```\na\n```', warnings: { tableCode: 1 } }
polymark.clean('<p style="color:red"><b>Жирный</b> текст</p><script>x()</script>');
// { result: '<p><strong>Жирный</strong> текст</p>' }
```

Swift:

```swift
import Polymark

let converted = try Polymark.transform("**Жирный**", from: .markdown, to: .html) // result: "<p><strong>Жирный</strong></p>"
```

Форматы на входе: `plain`, `markdown`, `html`, `rich`; на выходе к ним добавляется `telegram`.

Отказ — код ошибки: `length`, `unicode`, `structure`, `table`, `htmlLimit`, `representation`, `output`, `unsupported`, `format`. JavaScript возвращает его в `{ error }`, Swift бросает `PolymarkError`, Rust возвращает `Failure`. Неверные аргументы дают `invalid_argument`.

## Автор

[Алексей Симоненко](https://github.com/meritt)

## Лицензия

MIT. Смотрите файл `LICENSE`.
