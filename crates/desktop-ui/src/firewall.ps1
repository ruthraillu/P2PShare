# Invoked by the portable app using a UTF-16 encoded command. No profile is loaded.
# This file only defines functions; policy changes require -Apply $true explicitly.
function Invoke-P2PFirewall {
    param([Parameter(Mandatory=$true)][string]$Program, [bool]$Apply=$false)
    $ErrorActionPreference='Stop'
    $digest=[Security.Cryptography.SHA256]::Create()
    try { $hash=[BitConverter]::ToString($digest.ComputeHash([Text.Encoding]::UTF8.GetBytes($Program.ToLowerInvariant()))).Replace('-','').Substring(0,24) }
    finally { $digest.Dispose() }
    $name='P2PShare-UDP-v1-'+$hash
    function Get-AppRules([string]$Store) {
        @(Get-NetFirewallApplicationFilter -PolicyStore $Store | Where-Object {
            $_.Program -and [Environment]::ExpandEnvironmentVariables([string]$_.Program) -ieq $Program
        } | ForEach-Object { Get-NetFirewallRule -AssociatedNetFirewallApplicationFilter $_ })
    }
    function Test-UdpRule($Rule) {
        $port=Get-NetFirewallPortFilter -AssociatedNetFirewallRule $Rule
        [string]$port.Protocol -in @('UDP','17','Any','256')
    }
    function Test-OurProfiles($Rule) {
        $profiles=[int]$Rule.Profile
        $profiles -eq 0 -or ($profiles -band 7) -ne 0
    }
    if ($Apply) {
        # PersistentStore is local policy only. Never delete managed/global/other-app rules.
        $existing=Get-NetFirewallRule -PolicyStore PersistentStore -Name $name -ErrorAction SilentlyContinue
        if ($existing) {
            $app=Get-NetFirewallApplicationFilter -AssociatedNetFirewallRule $existing
            if ([Environment]::ExpandEnvironmentVariables([string]$app.Program) -ine $Program -or $existing.Group -ne 'P2PShare') { throw 'Firewall rule name conflict; no unrelated rule was changed.' }
            Remove-NetFirewallRule -InputObject $existing
        }
        $local=@(Get-AppRules 'PersistentStore')
        New-NetFirewallRule -PolicyStore PersistentStore -Name $name -DisplayName 'P2PShare - peer media (UDP)' -Group 'P2PShare' `
            -Description 'Allows encrypted P2PShare media and connectivity checks for this executable on Domain, Private and Public networks.' `
            -Program $Program -Direction Inbound -Action Allow -Enabled True -Profile Any -Protocol UDP `
            -LocalPort Any -RemotePort Any -LocalAddress Any -RemoteAddress Any | Out-Null
        foreach ($rule in $local) {
            if ($rule.Name -ne $name -and [string]$rule.Direction -eq 'Inbound' -and [string]$rule.Action -eq 'Block' -and (Test-UdpRule $rule) -and (Test-OurProfiles $rule)) {
                Remove-NetFirewallRule -InputObject $rule
            }
        }
    }
    if (-not $Apply) {
        try {
            $active=@(Get-AppRules 'ActiveStore')
            $profileList=@(Get-NetFirewallProfile -PolicyStore ActiveStore)
        } catch {
            $active=@(Get-AppRules 'PersistentStore')
            $profileList=@(Get-NetFirewallProfile -PolicyStore PersistentStore)
        }
    } else {
        $active=@(Get-AppRules 'ActiveStore')
        $profileList=@(Get-NetFirewallProfile -PolicyStore ActiveStore)
    }
    # Respect an existing Windows/user-created allowance too; do not ask again
    # merely because its name differs from this app's owned rule.
    $coverage=0
    $allows=@($active | Where-Object { [string]$_.Enabled -eq 'True' -and [string]$_.Action -eq 'Allow' -and [string]$_.Direction -eq 'Inbound' })
    foreach ($allow in $allows) {
        $port=Get-NetFirewallPortFilter -AssociatedNetFirewallRule $allow
        $address=Get-NetFirewallAddressFilter -AssociatedNetFirewallRule $allow
        $ifaceType=Get-NetFirewallInterfaceTypeFilter -AssociatedNetFirewallRule $allow
        $iface=Get-NetFirewallInterfaceFilter -AssociatedNetFirewallRule $allow
        $profiles=[int]$allow.Profile
        $usable=([string]$port.Protocol -in @('UDP','17','Any','256')) -and [string]$port.LocalPort -eq 'Any' -and [string]$port.RemotePort -eq 'Any' -and [string]$address.LocalAddress -eq 'Any' -and [string]$address.RemoteAddress -eq 'Any' -and [string]$ifaceType.InterfaceType -eq 'Any' -and [string]$iface.InterfaceAlias -eq 'Any'
        if ($usable) { if ($profiles -eq 0) { $coverage=$coverage -bor 7 } else { $coverage=$coverage -bor ($profiles -band 7) } }
    }
    $configured=($coverage -band 7) -eq 7
    $blocks=@($active | Where-Object { [string]$_.Enabled -eq 'True' -and [string]$_.Action -eq 'Block' -and [string]$_.Direction -eq 'Inbound' -and (Test-UdpRule $_) -and (Test-OurProfiles $_) })
    $restricted=@($profileList | Where-Object { [string]$_.Name -in @('Domain','Private','Public') -and [string]$_.Enabled -eq 'True' -and ([string]$_.AllowInboundRules -eq 'False' -or [string]$_.AllowLocalFirewallRules -eq 'False') })
    [pscustomobject]@{ configured=([bool]$configured -and $blocks.Count -eq 0 -and $restricted.Count -eq 0); app_udp_blocks=$blocks.Count; restricted_profiles=$restricted.Count; rule=$name } | ConvertTo-Json -Compress
}
