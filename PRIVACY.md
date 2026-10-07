# 隐私与登录凭据

IUsage 是非官方用量显示工具。程序需要联网查询额度，不是离线工具。没有开发者接收数据的服务器，不含统计上报或自动更新。

## Windows 登录方式

- **Claude 网页登录**：适合没有安装 Claude Code 的用户。手动提供 sessionKey；组织 ID 通常自动发现，多个组织时才需指定。单独登录 Claude 桌面客户端不能替代这两种配置。
- **Claude Code 登录**：读取本机 `~/.claude/.credentials.json`；访问令牌过期时可续期并写回该文件。
- **Codex**：读取本机 `~/.codex/auth.json`，不修改其令牌。

sessionKey 是敏感的账号会话凭据，并非仅能查用量的 API key。IUsage 仅用它查询组织和用量，但拿到它的其他程序或人员可能进行账号操作。不要发送给开发者、评论区、Issue 或陌生人。

## 数据流向

| 域名 | 用途 | 发送的凭据 |
|---|---|---|
| `claude.ai` | 网页渠道查询组织和用量 | 网页 sessionKey |
| `api.anthropic.com` | Claude Code 渠道查询用量 | Claude 访问令牌 |
| `platform.claude.com` | Claude 令牌续期 | Claude 刷新令牌 |
| `chatgpt.com` | Codex 用量查询 | Codex 访问令牌及所需账号标识 |

请求使用 HTTPS，系统验证证书，不跟随重定向；使用系统代理。应用不主动读取或上传聊天内容，不扫描浏览器 Cookie 数据库。上述行为可在 `src/providers.rs`、`src/providers/web.rs` 和 `src/win/http.rs` 查看。

## 本机保存与删除

Windows 数据位于 `%APPDATA%\IUsage\`：设置、用量缓存及等待状态、最近错误，以及 DPAPI 加密的 `claude-web.dpapi`。sessionKey 不写入普通设置或错误日志。DPAPI 保护磁盘上的副本，但无法防止同一 Windows 用户下的恶意程序读取凭据或进程内存。

“Claude 登录设置…”中的“清除登录信息”删除加密网页凭据，并切换回 Claude Code；“改用本机 Claude Code 登录”只切换渠道，保留网页凭据以便下次使用。清除本机副本不等于撤销网页会话；如怀疑泄露，应在官方账号页面终止相关会话。

卸载时先关闭开机自启并退出，删除 exe；需要清理本地数据时，再删除 `%APPDATA%\IUsage\`。不要为了绕过限流而清理缓存。

macOS 凭据和钥匙串行为见 [macOS 说明](macos/README.md)，目前不提供面向普通用户的 Mac 安装包。
