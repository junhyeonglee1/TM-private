[CmdletBinding()]
param(
    [string]$OutputPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $scriptDirectory '..\assets\stock-catalog.json'
}

$sourceDirectory = Join-Path ([IO.Path]::GetTempPath()) 'tm-stock-catalog-sources'
New-Item -ItemType Directory -Force -Path $sourceDirectory | Out-Null

$krxPath = Join-Path $sourceDirectory 'krx-listed.xls'
$nasdaqPath = Join-Path $sourceDirectory 'nasdaqlisted.txt'
$otherPath = Join-Path $sourceDirectory 'otherlisted.txt'

$sources = @(
    @{
        Uri = 'https://kind.krx.co.kr/corpgeneral/corpList.do?method=download&searchType=13'
        Path = $krxPath
    },
    @{
        Uri = 'https://www.nasdaqtrader.com/dynamic/SymDir/nasdaqlisted.txt'
        Path = $nasdaqPath
    },
    @{
        Uri = 'https://www.nasdaqtrader.com/dynamic/SymDir/otherlisted.txt'
        Path = $otherPath
    }
)

foreach ($source in $sources) {
    Invoke-WebRequest -UseBasicParsing -Uri $source.Uri -OutFile $source.Path
}

function Remove-HtmlMarkup {
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Value)

    $withoutTags = [regex]::Replace($Value, '<[^>]+>', ' ')
    $decoded = [Net.WebUtility]::HtmlDecode($withoutTags)
    return [regex]::Replace($decoded, '\s+', ' ').Trim()
}

function Format-UsSecurityName {
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Value)

    $name = $Value.Trim()
    $suffixes = @(
        '\s+-\s+Common Stock$',
        '\s+Common Stock$',
        '\s+-\s+Ordinary Shares$',
        '\s+Ordinary Shares$'
    )
    foreach ($suffix in $suffixes) {
        $name = [regex]::Replace($name, $suffix, '', 'IgnoreCase')
    }
    return $name.Trim()
}

$catalogItems = [Collections.Generic.List[object]]::new()
$seenSymbols = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$krxMainMarket = -join @([char]0xC720, [char]0xAC00)
$krxKosdaqMarket = -join @([char]0xCF54, [char]0xC2A4, [char]0xB2E5)

$krxEncoding = [Text.Encoding]::GetEncoding(51949)
$krxHtml = $krxEncoding.GetString([IO.File]::ReadAllBytes($krxPath))
$krxRows = [regex]::Matches($krxHtml, '<tr[^>]*>(.*?)</tr>', 'Singleline')
foreach ($row in $krxRows) {
    $cells = [regex]::Matches($row.Groups[1].Value, '<td[^>]*>(.*?)</td>', 'Singleline')
    if ($cells.Count -lt 3) {
        continue
    }
    $name = Remove-HtmlMarkup $cells[0].Groups[1].Value
    $segment = Remove-HtmlMarkup $cells[1].Groups[1].Value
    $ticker = Remove-HtmlMarkup $cells[2].Groups[1].Value
    $symbol = "KRX:$ticker"
    if (
        $segment -notin @($krxMainMarket, $krxKosdaqMarket) -or
        $ticker -notmatch '^\d{6}$' -or
        [string]::IsNullOrWhiteSpace($name) -or
        -not $seenSymbols.Add($symbol)
    ) {
        continue
    }
    $catalogItems.Add([ordered]@{
        market = 'KRX'
        ticker = $ticker
        name = $name
    })
}

$nasdaqRows = Import-Csv $nasdaqPath -Delimiter '|'
foreach ($row in $nasdaqRows) {
    $ticker = $row.Symbol.Trim().ToUpperInvariant()
    $name = Format-UsSecurityName $row.'Security Name'
    $symbol = "NASDAQ:$ticker"
    if (
        $row.'Test Issue' -ne 'N' -or
        $row.ETF -ne 'N' -or
        $ticker -notmatch '^[A-Z0-9.-]{1,10}$' -or
        [string]::IsNullOrWhiteSpace($name) -or
        -not $seenSymbols.Add($symbol)
    ) {
        continue
    }
    $catalogItems.Add([ordered]@{
        market = 'NASDAQ'
        ticker = $ticker
        name = $name
    })
}

$otherRows = Import-Csv $otherPath -Delimiter '|'
foreach ($row in $otherRows) {
    $market = switch ($row.Exchange) {
        'N' { 'NYSE' }
        'A' { 'AMEX' }
        default { $null }
    }
    if ($null -eq $market) {
        continue
    }
    $ticker = $row.'ACT Symbol'.Trim().ToUpperInvariant()
    $name = Format-UsSecurityName $row.'Security Name'
    $symbol = "${market}:$ticker"
    if (
        $row.'Test Issue' -ne 'N' -or
        $row.ETF -ne 'N' -or
        $ticker -notmatch '^[A-Z0-9.-]{1,10}$' -or
        [string]::IsNullOrWhiteSpace($name) -or
        -not $seenSymbols.Add($symbol)
    ) {
        continue
    }
    $catalogItems.Add([ordered]@{
        market = $market
        ticker = $ticker
        name = $name
    })
}

$orderedItems = @(
    $catalogItems |
        Sort-Object @{ Expression = {
            switch ($_.market) {
                'KRX' { 0 }
                'NASDAQ' { 1 }
                'NYSE' { 2 }
                default { 3 }
            }
        } }, @{ Expression = 'name' }, @{ Expression = 'ticker' }
)

$generatedAt = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
$catalog = [ordered]@{
    version = 1
    generatedAt = $generatedAt
    description = 'TM stock name-search catalog. Prices and portfolio data are not included.'
    sources = @(
        [ordered]@{
            name = 'KRX KIND listed-company directory'
            url = $sources[0].Uri
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $krxPath).Hash.ToLowerInvariant()
        },
        [ordered]@{
            name = 'Nasdaq Trader Nasdaq-listed securities'
            url = $sources[1].Uri
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $nasdaqPath).Hash.ToLowerInvariant()
        },
        [ordered]@{
            name = 'Nasdaq Trader other-listed securities'
            url = $sources[2].Uri
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $otherPath).Hash.ToLowerInvariant()
        }
    )
    counts = [ordered]@{
        KRX = @($orderedItems | Where-Object market -eq 'KRX').Count
        NASDAQ = @($orderedItems | Where-Object market -eq 'NASDAQ').Count
        NYSE = @($orderedItems | Where-Object market -eq 'NYSE').Count
        AMEX = @($orderedItems | Where-Object market -eq 'AMEX').Count
        total = $orderedItems.Count
    }
    items = $orderedItems
}

$resolvedOutputPath = [IO.Path]::GetFullPath($OutputPath)
$outputDirectory = Split-Path -Parent $resolvedOutputPath
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
$json = $catalog | ConvertTo-Json -Depth 8 -Compress
[IO.File]::WriteAllText($resolvedOutputPath, $json, [Text.UTF8Encoding]::new($false))

Write-Host "Stock catalog written: $resolvedOutputPath"
Write-Host (
    'Counts: KRX={0}, NASDAQ={1}, NYSE={2}, AMEX={3}, total={4}' -f
        $catalog.counts.KRX,
        $catalog.counts.NASDAQ,
        $catalog.counts.NYSE,
        $catalog.counts.AMEX,
        $catalog.counts.total
)
