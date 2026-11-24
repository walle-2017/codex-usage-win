# Security Policy

## Supported version

The latest published stable release is supported.

## Reporting a vulnerability

Please report security-sensitive issues privately when possible rather than posting credentials, tokens, account identifiers, or diagnostic logs in a public issue.

Repository: https://github.com/walle-2017/codex-usage-win

## Credential handling

Codex Usage Win reads credentials already managed by Codex. It does not automatically invoke the Codex CLI to refresh credentials and does not send Codex credentials to Walle Studio.

Never include `auth.json`, access tokens, account IDs, or unredacted diagnostic material in bug reports.
