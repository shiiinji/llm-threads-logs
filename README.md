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
    ],
    "SessionEnd": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "review_session"
          }
        ]
      }
    ]
  }
}
```

> **Note**: `Stop` hook は各ターン完了時に発火します。`SessionEnd` hook はセッション終了時に発火し、会話内容をレビューして新しい Skill を提案します。

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

各ターン完了時（Claude の応答終了時）に `$OBSIDIAN_VAULT/$OBSIDIAN_AI_ROOT/Claude Code/<project>/Threads/YYYY/mm/DD/` にMarkdownが生成・更新される。

### Codex CLI

エージェントのターン完了時に `$OBSIDIAN_VAULT/$OBSIDIAN_AI_ROOT/Codex/<project>/Threads/YYYY/mm/DD/` にMarkdownが追記される。

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

5. **（Codex CLI）プロジェクトの trust 設定を確認**
   - Codex が「untrusted」扱いのディレクトリでは、バージョン/設定によって `notify` コマンドの実行が抑止されることがあります。
   - その場合は `~/.codex/config.toml` に対象プロジェクトを追加して `trust_level = "trusted"` にしてください。

6. **（Codex CLI / fnm）Node.js のバージョン差分を疑う**
   - `npm i -g @openai/codex` の場合、`codex` は `#!/usr/bin/env node` で起動するため、ディレクトリごとに `fnm` が `node` を切り替える構成だと挙動が変わる可能性があります。
   - `which codex` / `codex --version` / `node -v` を、ログが出るディレクトリと出ないディレクトリで比較してください。

## Makefile コマンド

| コマンド | 説明 |
|----------|------|
| `make build` | デバッグビルド |
| `make release` | リリースビルド |
| `make install` | リリースビルド後、`~/.local/bin` へコピー |
| `make uninstall` | バイナリを削除 |
| `make obsidian-dirs` | 出力先ディレクトリを作成 |

## 生成されるバイナリ

- `claude_session_to_obsidian` — Claude Code の Stop hook: stdin JSON → Markdown 生成
- `codex_notify_to_obsidian` — Codex CLI notify: argv[1] JSON → Markdown 追記
- `review_session` — Claude Code の SessionEnd hook: 会話内容をレビューし Skill 提案を生成

## Skill 提案機能

`review_session` はセッション終了時に自動実行され、以下を行います：

1. 作成された MD ファイルから User の指示を抽出
2. `codex exec -c 'notify=[]'` で LLM にレビューさせる
3. 再利用可能な Skill パターンを提案
4. 提案を `$OBSIDIAN_VAULT/$OBSIDIAN_AI_ROOT/skill_proposals/` に保存
