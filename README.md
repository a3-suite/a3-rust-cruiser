# a3-rust-cruiser

JavaScript/TypeScript向けの[dependency-cruiser](https://github.com/sverweij/dependency-cruiser)に着想を得て、Rust版dependency-cruiserを目指している論理モジュール境界検証CLIです。Cargo packageとcompilation targetを1つずつ解析し、依存方向と循環依存をプロジェクト固有のポリシーに対して検証します。

## できること

- Rustの内部モジュールと、`use`、re-export、パス参照による依存を検出する
- `allowed`、`forbidden`、`circular`の3種類のルールで依存方向と循環依存を検証する
- `*`と`**`を使った論理モジュールパターンでルールの対象を指定する
- feature、cfg、target triple、解析対象モジュールを実行単位ごとに指定する
- 検証結果をtextまたはJSONで出力し、`conformant`、`non-conformant`、`indeterminate`を区別する
- YAMLポリシーを直接使用するほか、ownerが生成した解決済み実装帰属を検証条件として直接使用する

## できないこと

- crate間またはworkspace package間の依存を検証する
- 複数のCargo packageやcompilation targetを1回の実行でまとめて検証する
- 依存グラフを画像や対話的な図として可視化する
- macro expansionや名前解決をRustコンパイラと同等に完全解決する。既定の`compilation` profileではCargo feature、target dependency、rustcのtarget cfg、既知macroと字句scopeを解析し、残る解析範囲外の構造が境界判定へ影響し得る場合だけ`indeterminate`になる。`source-union` profileは同じ判定保証を提供しない
- dependency-cruiserの設定ファイルやルールとの互換性を提供する

## 前提

- 開発・CIで検証するRust toolchainは1.98.0（`rustc` / `cargo` 1.98.0、`rust-toolchain.toml`で固定）。これはMSRVの宣言ではありません

## CLI情報

```sh
a3-rust-cruiser --version
a3-rust-cruiser --help
a3-rust-cruiser check --help
```

GitHub Releaseの固定版セットアップとchecksum検証は、`skills/a3-rust-cruiser`の公開スキルを使用します。

## ポリシーの作成

通常のRustプロジェクトでは、YAML形式のboundary policyをプロジェクト内に作成します。SDDの導入やSDDファイルの配置は必要ありません。

1. `crate::application`や`crate::domain`のように、検証する論理モジュール境界を決めます。
2. 許可する依存方向、禁止する依存方向、循環を禁止する範囲を決めます。
3. 決定した境界を`boundary-policy.yaml`へ記述します。

```yaml
rules:
  - id: allow-application-to-domain
    kind: allowed
    severity: error
    from: [crate::application]
    to: [crate, crate::domain]
  - id: forbid-domain-to-application
    kind: forbidden
    severity: error
    from: [crate::domain]
    to: [crate::application]
  - id: prevent-component-cycles
    kind: circular
    severity: error
    modules: [crate::application, crate::domain]
```

- `allowed`: `from`に一致するモジュールから、`to`に一致するモジュールへの依存だけを許可します。
- `forbidden`: `from`から`to`への依存を禁止します。`allowed`にも一致する場合は`forbidden`を優先します。
- `circular`: `modules`に一致するモジュール間の循環依存を禁止します。
- `id`: ポリシー内で重複しない小文字kebab-caseを指定します。
- `allowed`と`forbidden`には`from`と`to`を指定し、`modules`は指定しません。`circular`には`modules`を指定し、`from`と`to`は指定しません。
- `severity`: `error`または`warning`を指定します。どちらも違反があれば`non-conformant`になり、severityはレポート上の分類として使用されます。
- `dependency_kinds`: 必要な場合だけ`import`、`re-export`、`path-reference`から対象とする依存種別を指定します。
- モジュールパターンの`*`は1階層、`**`は0階層以上に一致します。

同じポリシーは[examples/minimal-project/boundary-policy.yaml](examples/minimal-project/boundary-policy.yaml)にもあります。

境界入力は、YAML boundary policyを`--policy`で指定するか、ownerが生成した`resolved-implementation-attribution` version 1を`--resolved-implementation-attribution`で指定します。2つの入力は相互排他です。解決済み実装帰属は中間変換せず、ownerが確定したfile attributionと依存関係を使用します。

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

## ライセンス

[MIT License](LICENSE) で公開します。
