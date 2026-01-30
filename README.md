# llms-threads-logs

Obsidian Vault に AI の会話ログを自動で Markdown 化して保存するためのバイナリ集です。

## Build & Install

```bash
# ビルド & インストール
make install

# ~/.local/bin にパスが通っていることを確認
which claude_session_to_obsidian
which codex_notify_to_obsidian
```

## セットアップ手順

### 1. 環境変数の設定（必須）

`~/.zshrc` (または `~/.bashrc`) に以下を追加：

```bash
# ai-log-exporter: Obsidian出力先
export OBSIDIAN_VAULT="/Users/YOURNAME/Documents/llm-threads"
export OBSIDIAN_AI_ROOT="llms"
```

追加後、シェルを再起動するか `source ~/.zshrc` を実行。

### 2. Claude Code の hook 設定

`~/.claude/settings.json` に以下を追加：

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "claude_session_to_obsidian"
          }
        ]
      }
    ]
  }
}
```

> **Note**: `Stop` hook は各ターン完了時に発火します。セッション終了時のみにしたい場合は `Stop` を `SessionEnd` に変更してください。

### 3. Codex CLI の notify 設定

`~/.codex/config.toml` の `notify` を以下のように設定：

```toml
notify = ["bash", "-lc", "codex_notify_to_obsidian \"$1\" && afplay /System/Library/Sounds/Ping.aiff", "--"]
```

（サウンドが不要なら `&& afplay ...` 部分を削除）

### 4. 出力先ディレクトリの作成

```bash
make obsidian-dirs
```

## 動作確認

### Claude Code

各ターン完了時（Claude の応答終了時）に `$OBSIDIAN_VAULT/$OBSIDIAN_AI_ROOT/Claude Code/<project>/Threads/` にMarkdownが生成・更新される。

### Codex CLI

エージェントのターン完了時に `$OBSIDIAN_VAULT/$OBSIDIAN_AI_ROOT/Codex/<project>/Threads/` にMarkdownが追記される。

## トラブルシューティング

ログが書き込まれない場合：

1. **環境変数の確認**
   ```bash
   echo $OBSIDIAN_VAULT
   echo $OBSIDIAN_AI_ROOT
   ```

2. **バイナリの確認**
   ```bash
   which claude_session_to_obsidian
   which codex_notify_to_obsidian
   ```

3. **手動テスト（Claude Code）**
   ```bash
   echo '{"session_id":"test","transcript_path":"/path/to/transcript.jsonl","cwd":"/tmp"}' | claude_session_to_obsidian
   ```

4. **hook設定の確認**
   - Claude Code: `~/.claude/settings.json`
   - Codex CLI: `~/.codex/config.toml`

## Makefile コマンド

| コマンド | 説明 |
|----------|------|
| `make build` | デバッグビルド |
| `make release` | リリースビルド |
| `make install` | リリースビルド後、`~/.local/bin` へコピー |
| `make uninstall` | バイナリを削除 |
| `make obsidian-dirs` | 出力先ディレクトリを作成 |

## 生成されるバイナリ

- `claude_session_to_obsidian` — Claude Code の SessionEnd hook: stdin JSON → Markdown 生成
- `codex_notify_to_obsidian` — Codex CLI notify: argv[1] JSON → Markdown 追記
