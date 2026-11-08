# Implementation plan

1. Inspect existing Outlook, Google, Teams and slackcli contracts.
2. Add account routing, signed resource handles and minimal adapters.
3. Replace MCP registrations without old aliases.
4. Test explicit account isolation, schema restrictions, CLI injection, provider failures and pagination.
5. Run TypeScript, unit tests and Docker image integration.
6. Provision approved existing Slack profile outside deployment source.
7. Capture rollback, deploy tested image through existing workflow and verify live reads.
8. Record evidence and clean owned scratch files.
