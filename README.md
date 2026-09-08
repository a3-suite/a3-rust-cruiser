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

通常のRustプロジェクト向けの作成手順・YAML記述例は、公開スキルの[BoundaryPolicy作成ガイド](skills/a3-rust-cruiser/references/create-boundary-policy.guide.md)を参照してください。実行用の最小例は[examples/minimal-project/boundary-policy.yaml](examples/minimal-project/boundary-policy.yaml)にあります。

SDDを正本として運用するプロジェクトでは、`boundary-policy.yaml`を作成・使用せず、SDD ownerが生成した`resolved-implementation-attribution` version 1を唯一の境界入力として使用します。`--policy`は、SDDを使用しない通常のRustプロジェクト向けです。

境界入力は、通常のプロジェクトではYAML boundary policyを`--policy`で指定し、SDD対応プロジェクトではownerが生成した`resolved-implementation-attribution` version 1を`--resolved-implementation-attribution`で指定します。2つの入力は相互排他です。解決済み実装帰属は中間変換せず、ownerが確定したfile attributionと依存関係を使用します。

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
