# YAML BoundaryPolicy作成ガイド

## 目的

- 非SDDのRustプロジェクトで決定した境界ルールを、a3-rust-cruiserの入力となるYAML BoundaryPolicyへ記述する。

## 前提条件

- 利用者がpolicyの作成または修正を依頼していること。検証だけの依頼から自動作成しない。
- SDDを正本として運用するプロジェクトでは使用しない。SDD出力が不足している場合もpolicyで代替せず、a3-sddへ委譲する。

## 作成フロー

1. 対象プロジェクトの設計方針とRustのモジュール宣言を確認し、検証する論理モジュールと許可・禁止する依存方向、循環を禁止する範囲を確定する。現状の依存をそのまま許可ルールにせず、方針が不明な部分は利用者へ確認する。
2. プロジェクト内の保存先を確定する。新規の場合は`boundary-policy.yaml`を候補とし、既存policyがある場合は内容と変更目的を確認して必要なルールだけ修正する。
3. 下の記述例を対象プロジェクトの論理モジュール名と方針に合わせて編集する。ファイルパスではなく`crate::application`のようなモジュールIDを使い、子モジュールも対象にする場合はpatternの範囲を明示する。
4. 記述した各ルールを手順1の方針へ照合する。解析コンテキストや解析結果はYAMLへ加えない。
5. references/run-boundary-policy-check.guide.mdへ進み、作成したファイルを`--policy`へ渡す。CLIが未導入なら作成済み・検証未実施を報告し、セットアップは別途承認を得る。

## 記述例

次は`crate::application`と`crate::domain`を持つプロジェクトの例であり、対象の設計方針を自動的に決定するものではない。

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

- `allowed`: `from`に一致するモジュールの許可先を`to`で指定する。同じ依存元に適用される複数の`allowed`は許可先の和集合として評価される。
- `forbidden`: `from`から`to`への依存を禁止する。`allowed`にも一致する場合は`forbidden`を優先する。
- `circular`: `modules`に一致するモジュール間の循環依存を禁止する。
- `id`: ポリシー内で重複しない小文字kebab-caseを指定する。
- `allowed`と`forbidden`には空でない`from`と`to`を指定し、`modules`は指定しない。`circular`には空でない`modules`を指定し、`from`と`to`は指定しない。
- `severity`: `error`または`warning`を指定する。どちらも違反があれば`non-conformant`になり、severityはレポート上の分類に使われる。
- `dependency_kinds`: 依存種別を限定する場合だけ`import`、`re-export`、`path-reference`の配列を指定する。省略時は全種別を対象とする。
- モジュールpatternの`*`は1階層、`**`は0階層以上に一致する。`crate::domain`はそのモジュールだけ、`crate::domain::**`は自身と子孫を対象にする。

## 完了確認

- 保存先、境界ルールの根拠、検証した解析コンテキストと判定を報告する。ファイル作成と境界検証の完了は分ける。
- 入力エラーならYAMLと診断を照合する。`non-conformant`なら設計方針と実装の不一致を確認し、通過させるためだけにルールを緩めない。`indeterminate`や未実行を検証完了にしない。
