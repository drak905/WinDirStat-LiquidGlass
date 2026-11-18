param([Parameter(ValueFromRemainingArguments = $true)][string[]] $RemainingArgs)
$responder = Join-Path (Split-Path -Parent $PSScriptRoot) "mock_agent_responder.mjs"
& node $responder --provider opencode @RemainingArgs
exit $LASTEXITCODE
