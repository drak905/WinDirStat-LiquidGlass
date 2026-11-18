param([Parameter(ValueFromRemainingArguments = $true)][string[]] $RemainingArgs)
$responder = Join-Path (Split-Path -Parent $PSScriptRoot) "mock_agent_responder.mjs"
& node $responder --provider antigravity @RemainingArgs
exit $LASTEXITCODE
