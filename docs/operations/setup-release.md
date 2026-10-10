# GitHub Release オフライン導入

## 対象

オンライン導入と起動確認は `README.md` を参照してください。この文書では、事前取得した manifest と CLI archive を使うオフライン導入を説明します。製品固有の asset 宣言は `installer/assembly.json` と `installer/manifests/` が正本です。

## オフライン導入

事前取得した manifest と CLI archive を使い、対象 platform 用 installer を実行します。Unix の共有 wrapper は使用せず、対象別 installer を指定します。

Linux x86_64:

```sh
bash ./install-linux-x86_64.sh --source offline --manifest <path> --artifact <path>
```

macOS arm64:

```sh
bash ./install-macos-arm64.sh --source offline --manifest <path> --artifact <path>
```

Windows x86_64:

```powershell
powershell -ExecutionPolicy Bypass -NoProfile -File .\install.ps1 -Source offline -Manifest <path> -Artifact <path>
```

Offline mode はネットワークから入力を取得しません。
