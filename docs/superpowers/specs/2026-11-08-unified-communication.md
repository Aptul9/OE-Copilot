# Unified communication contract

Approved replacement of MCP surface. No compatibility aliases. Every data operation requires exact account_id. Inventory includes provider, native account label and capabilities. Resource handles carry signed account, kind and parent context. A handle from another account is refused before data access. Stable production signing key comes from existing deployment credential, without committing it.

## Providers

Outlook and Gmail share email tools. Teams, Google Chat and Slack share chat tools. Calendar reads remain explicit-account Outlook operations. Unsupported features return UNSUPPORTED_CAPABILITY. Google requests use existing native agent APIs. Teams uses existing MCP relay connection. Slack uses pinned slackcli 0.19.1 as a server subprocess, JSON output, explicit workspace and bounded execution. No admin, canvas or saved-item tools.

## Safety

No default account, fragment matching or cross-account search at the MCP boundary. Strict tool schemas reject unknown arguments and local filesystem paths. Slack commands are allowlisted arrays, shell false, 45-second timeout, 8 MB output bound. Flags use equals-form values; search uses an option terminator. Credentials remain outside source and Docker build context. No public test sends.

## Deployment

Single existing MCP deployment. Slack profile mounted read-only from /opt/communication-mcp-state/slackcli. Image installs checksum-pinned Linux CLI. Prior deployment SHA and image recorded before rollout. Existing workflow retains previous source archive.
