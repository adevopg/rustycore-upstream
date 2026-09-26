<#
    start-shop.ps1 - Segunda instancia de la tienda web (LegionCore\shop\shop-server.mjs) para RustyCore.

    Reutiliza el mismo codigo, el mismo certificado (tienda.nightspire.gg, Let's Encrypt) y las mismas
    claves de SumUp que la tienda de LegionCore (se cargan de LegionCore\shop\shop.env), pero apunta a
    las bases rc_* y escucha en el puerto 8096. La tienda de LegionCore sigue en el 8095 sin tocar.
#>
$ErrorActionPreference = 'Stop'
$shopDir = 'C:\Users\Administrator.WIN-Q58DL6IGT95\Desktop\LegionCore\shop'
$runtime = 'C:\Users\Administrator.WIN-Q58DL6IGT95\Desktop\rustycore-runtime'

# 1. Variables base de la tienda de LegionCore (incluye SumUp, SMTP, etc.)
Get-Content "$shopDir\shop.env" | Where-Object { $_ -match '^\s*[A-Z_]+=' } | ForEach-Object {
    $k, $v = $_ -split '=', 2
    [Environment]::SetEnvironmentVariable($k.Trim(), $v.Trim().Trim('"'), 'Process')
}

# 2. Lo que cambia para RustyCore
$env:SHOP_PORT           = '8096'
$env:SHOP_BIND           = '0.0.0.0'
$env:SHOP_PUBLIC_URL     = 'https://tienda.nightspire.gg:8096'
$env:SHOP_NAME           = 'Tienda NightSpire Classic'
$env:SHOP_LOG            = "$runtime\logs\shop.log"
$env:SHOP_DB_HOST        = '127.0.0.1'
$env:SHOP_DB_PORT        = '3306'
$env:SHOP_DB_AUTH        = 'rc_auth'
$env:SHOP_DB_WORLD       = 'rc_world'
$env:SHOP_DB_CHARACTERS  = 'rc_characters'
$env:SHOP_DB_USER        = 'rustycore'
$env:SHOP_DB_PASS        = '<password>'
$env:RAF_PUBLIC_URL      = 'https://tienda.nightspire.gg:8096'
$env:LEGION_DB_USER      = 'rustycore'
$env:LEGION_DB_PASS      = '<password>'

# 3. Parar una instancia anterior de ESTA tienda (la del 8095 no se toca)
Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object { $_.CommandLine -match 'shop-server\.mjs' } | ForEach-Object {
    $listen = Get-NetTCPConnection -OwningProcess $_.ProcessId -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalPort -eq 8096 }
    if ($listen) { Stop-Process -Id $_.ProcessId -Force }
}

$p = Start-Process -FilePath 'C:\Program Files\nodejs\node.exe' -WorkingDirectory $shopDir -ArgumentList 'shop-server.mjs' `
    -RedirectStandardOutput "$runtime\logs\shop.out" -RedirectStandardError "$runtime\logs\shop.err" -WindowStyle Hidden -PassThru
Write-Host "tienda RustyCore PID $($p.Id) en https://tienda.nightspire.gg:8096"
