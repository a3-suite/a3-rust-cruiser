# a3-rust-cruiser

JavaScript/TypeScript向けの[dependency-cruiser](https://github.com/sverweij/dependency-cruiser)に着想を得た、Rust向けの論理モジュール境界検証CLIです。Cargo packageとcompilation targetを1つずつ解析し、依存方向と循環依存をプロジェクト固有のポリシーに対して検証します。

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

## 導入

Linux x86_64、macOS arm64、Windows x86_64 に対応しています。installer はユーザー単位で配置し、PATH は自動変更しません。

### Linux・macOS

```sh
curl -fsSL --proto '=https' --proto-redir '=https' "https://github.com/a3-suite/a3-rust-cruiser/releases/download/v0.2.0/install.sh" | sh
```

managed root はLinuxが `~/.local/share/a3-suite/a3-rust-cruiser/standalone`、macOSが `~/Library/Application Support/a3-suite/a3-rust-cruiser/standalone` です。launcher はどちらも `~/.local/bin/a3-rust-cruiser` に配置されます。コマンド名で実行するには、現在のシェルで PATH に追加します。

```sh
export PATH="$HOME/.local/bin:$PATH"
```

### Windows

Windows PowerShell 5.1 以降で実行してください。

```powershell
powershell -ExecutionPolicy Bypass -NoProfile -Command "irm 'https://github.com/a3-suite/a3-rust-cruiser/releases/download/v0.2.0/install.ps1' | iex"
```

managed root は `%LOCALAPPDATA%\a3-suite\a3-rust-cruiser\standalone`、launcher は `%LOCALAPPDATA%\Programs\a3-suite\a3-rust-cruiser\bin\a3-rust-cruiser.exe` に配置されます。コマンド名で実行するには、現在の PowerShell セッションで PATH に追加して導入を確認します。

```powershell
$env:Path = "$env:LOCALAPPDATA\Programs\a3-suite\a3-rust-cruiser\bin;$env:Path"
```

PATH を設定したシェルで次を実行し、`0.2.0` と表示されることを確認してください。

```text
a3-rust-cruiser --version
```

オフライン導入は[セットアップ手順](docs/operations/setup-release.md)を参照してください。

### Agent Skillのインストール

本製品は対応スキルとセットで使用します。同じ版の[対応スキルZIP](https://github.com/a3-suite/a3-rust-cruiser/releases/download/v0.2.0/a3-rust-cruiser-skills.zip)と同じReleaseの`.sha256`をダウンロードし、SHA-256を照合してから展開して、利用者環境の適切なスキルフォルダに配置してください。削除する場合は、配備した`<skill-root>/a3-rust-cruiser/`だけを削除します。

### アンインストール

実行中のCLIとインストーラを終了してから削除してください。インストール先の全バージョンと管理情報を削除します。残したいファイルがある場合は事前に退避してください。

Linux:

```sh
rm -f "$HOME/.local/bin/a3-rust-cruiser"
rm -r "$HOME/.local/share/a3-suite/a3-rust-cruiser/standalone"
```

macOS:

```sh
rm -f "$HOME/.local/bin/a3-rust-cruiser"
rm -r "$HOME/Library/Application Support/a3-suite/a3-rust-cruiser/standalone"
```

Windows PowerShell:

```powershell
$launcher = "$env:LOCALAPPDATA\Programs\a3-suite\a3-rust-cruiser\bin\a3-rust-cruiser.exe"
$managedRoot = "$env:LOCALAPPDATA\a3-suite\a3-rust-cruiser\standalone"
if (Test-Path -LiteralPath $launcher) { Remove-Item -LiteralPath $launcher -Force }
if (Test-Path -LiteralPath $managedRoot) { Remove-Item -LiteralPath $managedRoot -Recurse -Force }
```

永続設定に追加した製品専用のPATH項目だけ外してください。共有の `~/.local/bin` は残してください。

## 使い方

導入したCLIで、Cargo projectの論理モジュール境界を検証します。検証対象のprojectを扱えるRust toolchain（`rustc` / `cargo`）が利用できる前提です。

```sh
a3-rust-cruiser check ./my-rust-project \
  --policy boundary-policy.yaml \
  --package my-package \
  --target bin:my-binary \
  --target-triple "$(rustc -vV | sed -n 's/^host: //p')" \
  --format text
```

`decision: conformant` と終了コード `0` が返れば適合です。`non-conformant` は `1`、`indeterminate` は `2`、入力不正や実行失敗は `3` を返します。

```sh
a3-rust-cruiser --help
a3-rust-cruiser check --help
```

## ポリシーの作成

通常のRustプロジェクト向けの作成手順・YAML記述例は、公開スキルの[BoundaryPolicy作成ガイド](skills/a3-rust-cruiser/references/create-boundary-policy.guide.md)を参照してください。実行用の最小例は[examples/minimal-project/boundary-policy.yaml](examples/minimal-project/boundary-policy.yaml)にあります。

SDDを正本として運用するプロジェクトでは`boundary-policy.yaml`を作成せず、SDD ownerが生成した`resolved-implementation-attribution` version 1を`--resolved-implementation-attribution`で指定します。`--policy`とは相互排他です。

## 関連情報

- [成果物の運用（導入・利用の詳細）](docs/operations/README.md)
- [ドキュメント一覧](docs/README.md)
- [実行ガイド（BoundaryPolicy入力）](skills/a3-rust-cruiser/references/run-boundary-policy-check.guide.md)
- [実行ガイド（resolved-implementation-attribution入力）](skills/a3-rust-cruiser/references/run-resolved-implementation-attribution-check.guide.md)
