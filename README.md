# a3-rust-cruiser

Cargo package と compilation target を1つずつ解析し、Rust の論理モジュール境界を検証する CLI です。

## 前提

- Rust 2024 edition に対応した Rust toolchain

## 最短実行

同梱の最小プロジェクトを検証します。

```sh
cargo run -- check examples/minimal-project \
  --policy examples/minimal-project/boundary-policy.yaml \
  --package minimal-project \
  --target bin:minimal-project \
  --target-triple "$(rustc -vV | sed -n 's/^host: //p')" \
  --format text
```

`decision: conformant` と終了コード `0` が返れば成功です。

## 正本

- 要求: [requirement.sdd.yml](sdd/dsl/requirements/items/requirements/validate-rust-logical-module-boundaries/requirement.sdd.yml)
- CLI仕様: [cli-command.sdd.yml](sdd/dsl/specs/cli/commands/check/cli-command.sdd.yml)
- 検証契約: [observations.sdd.yml](sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/observations.sdd.yml)
- DDD分析メモ: [logical-module-boundary-ddd-analysis.md](docs/notes/logical-module-boundary-ddd-analysis.md)

要求・仕様・設計の判断は SDD の DSL を正本とし、README では再定義しません。
