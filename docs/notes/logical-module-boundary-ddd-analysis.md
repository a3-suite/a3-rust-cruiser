# 論理モジュール境界検証の DDD 分析

## 位置づけ

この文書は、論理モジュール境界検証に関する非正本の分析メモである。
要求、品質条件、制約の正本は `sdd/dsl/requirements/` 配下の DSL とし、この文書では再定義しない。

分析入力:

- `sdd/dsl/requirements/items/requirements/validate-rust-logical-module-boundaries/requirement.sdd.yml`
- `sdd/dsl/requirements/items/use-cases/check-logical-module-boundaries/use-case.sdd.yml`
- `sdd/dsl/requirements/items/constraints/logical-module-analysis-boundary/constraint.sdd.yml`

## 結論

MVP は `Logical Module Boundary Validation` という単一の bounded context とする。
Rust の意味解析、境界評価、CLI や Cargo との接続を別 bounded context へ先行分割せず、同じ言葉と判断責務が通用する一つの境界内で capability を分ける。

別 bounded context への分割は、Rust 以外の言語対応、解析結果の独立公開、または各 capability の変更責任と公開契約が実際に分離した場合に再検討する。

## Domain capability

| Capability | 分類 | 責務 |
| --- | --- | --- |
| Boundary Evaluation | core domain | 観測された依存が境界ポリシーへ適合するか判断する |
| Rust Dependency Observation | supporting domain | Rust の名前解決結果を論理モジュールと型付き依存辺へ変換する |
| Project Integration | adapter | Cargo、ファイルシステム、CLI、CI、出力形式と接続する |

依存方向は `Project Integration -> use case -> Boundary Evaluation` とする。
`Rust Dependency Observation` は use case が要求する port を実装し、`Boundary Evaluation` は Cargo、構文木、ファイルシステム、CLI の型へ依存しない。

## ユビキタス言語

| 用語 | 意味 | 混同してはならないもの |
| --- | --- | --- |
| `AnalysisContext` | package、target、feature、`cfg`、target triple を含む解析条件 | 単なる実行時オプション一覧 |
| `LogicalModule` | Rust の名前空間上のモジュール | ファイル、ディレクトリ、Cargo package |
| `ModuleDeclaration` | `mod` による包含宣言 | 利用依存 |
| `DependencyEdge` | ある論理モジュールから別の論理モジュールへの意味的参照 | モジュール包含 |
| `DependencyKind` | 通常参照、import、re-export など依存の意味分類 | ソース構文の表記差だけによる分類 |
| `ObservedDependencyGraph` | 特定の `AnalysisContext` で観測されたモジュールと依存辺の不変 snapshot | ファイル一覧、Cargo package graph |
| `BoundaryPolicy` | 依存方向の許可、禁止、循環条件を表す判断規則 | CLI 設定形式 |
| `BoundaryViolation` | 観測された依存と `BoundaryPolicy` が矛盾した結果 | 解析不能、I/O 障害 |
| `SourceEvidence` | 依存判断の根拠となるソース位置と解決情報 | 表示用メッセージだけの文字列 |
| `AnalysisIssue` | 完全な依存観測を妨げた解決不能参照や未対応条件 | 境界違反 |
| `ValidationDecision` | 適合性に関する最終判断 | process の終了コード |

分析時に候補とした `ValidationDecision` の状態と外部表現は、昇格済みの契約条項と CLI 仕様を参照する。

## Domain model 候補

MVP は永続的な lifecycle を持つ業務 entity を必要としない。解析入力、snapshot、判定結果を value object として扱い、entity、repository、domain event を形式のために導入しない。

Value object 候補:

- `AnalysisContext`
- `LogicalModuleId`
- `DependencyEdge`
- `SourceEvidence`
- `ObservedDependencyGraph`
- `BoundaryRuleId`
- `BoundaryViolation`
- `AnalysisIssue`
- `ValidationDecision`

Policy または domain service 候補:

- `ClassifyDependency`
- `EvaluateBoundaryPolicy`
- `DetectDependencyCycles`
- `DecideValidationOutcome`

Application use case:

- `ValidateLogicalModuleBoundaries`

Port 候補:

- `ObserveRustDependencies`
- `LoadBoundaryPolicy`

Adapter 候補:

- inbound: CLI、CI
- outbound: Cargo metadata、Rust 意味解析器、ファイルシステム、境界ポリシー入力

## 昇格済みの判断

不変条件と検証可能な観測項目は正本へ昇格済みであり、このメモでは再定義しない。

- 要求: `../../sdd/dsl/requirements/items/requirements/validate-rust-logical-module-boundaries/requirement.sdd.yml`
- 契約（観測項目を含む）: `../../sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/subject.sdd.yml`
- 検証項目: `../../sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/verifications.sdd.yml`

## 失敗の分類

分析では domain outcome、domain invariant violation、技術的失敗を分離した。昇格済みの分類と責務境界は契約条項を参照する。

## 未確定事項

MVP 後に検討する解析範囲は、要求側の制約
`../../sdd/dsl/requirements/items/constraints/logical-module-analysis-boundary/constraint.sdd.yml`
を参照する。新しい判断が必要になった場合は、該当する SDD owner の DSL を更新し、このメモへ契約内容を複製しない。
