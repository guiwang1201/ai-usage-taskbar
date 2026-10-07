# 安全反馈

不要在公开 Issue、截图或私信中附上 sessionKey、访问令牌、刷新令牌、`auth.json`、`.credentials.json`、浏览器 Cookie 导出或整个配置目录。使用模拟值复现问题。

公开发布前，维护者应在 GitHub 的 Security 设置中启用 Private vulnerability reporting。启用后，请通过仓库 Security → Report a vulnerability 私下报告；若入口不可用，先提交不含漏洞细节和个人信息的 Issue 请求私下联系渠道，不要公开利用步骤或凭据。

报告可包含版本、Windows 版本、复现步骤、预期与实际行为。只提供经过脱敏的截图和错误文字。

发布二进制目前未签名。请从本项目 GitHub Releases 获取，并核对同版本的 SHA-256。校验和用于检查文件一致性，不能代替代码签名或独立安全审计。
