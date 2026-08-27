# podctl fish completion
function __pods_no_subcmd
    set -l cmd (commandline -opc)
    test (count $cmd) -lt 2
end

complete -c podctl -n __pods_no_subcmd -a "status battery ping mode conv spatial ear mic loud-reduction press tone-on-press rename one-bud-anc auto-anc chime connect disconnect pair unpair list auto-connect volume mute profile codec default latency watch meter tray popup reboot completion debug install uninstall version help" -d 'podctl command'

complete -c podctl -n '__fish_seen_subcommand_from mode'             -a 'off anc transparency adaptive'
complete -c podctl -n '__fish_seen_subcommand_from conv'             -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from spatial'          -a 'off fixed head-tracked'
complete -c podctl -n '__fish_seen_subcommand_from ear'              -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from mic'              -a 'auto left right'
complete -c podctl -n '__fish_seen_subcommand_from loud-reduction'   -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from tone-on-press'    -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from one-bud-anc'      -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from mute'             -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from auto-connect'     -a 'on off'
complete -c podctl -n '__fish_seen_subcommand_from profile'          -a 'high headset off'
complete -c podctl -n '__fish_seen_subcommand_from codec'            -a 'sbc aac aptx aptx_hd ldac'
complete -c podctl -n '__fish_seen_subcommand_from press'            -a 'left right mode-cycle siri none'
complete -c podctl -n '__fish_seen_subcommand_from meter'            -a '--plain --json --once --interval --device'
complete -c podctl -n '__fish_seen_subcommand_from tray'             -a 'start stop status restart'
complete -c podctl -n '__fish_seen_subcommand_from completion'       -a 'bash zsh fish'
complete -c podctl -n '__fish_seen_subcommand_from debug'            -a 'emit-case-lid --no-redact'
complete -c podctl -n '__fish_seen_subcommand_from emit-case-lid'    -a 'open close'
complete -c podctl -n '__fish_seen_subcommand_from install'          -a '--yes --no-daemon --no-completion --no-manpages --with-tray --with-popup'
complete -c podctl -n '__fish_seen_subcommand_from uninstall'        -a '--yes'
