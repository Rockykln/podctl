# podctl bash completion
_pods() {
    local cur prev words cword
    _init_completion || return

    local cmds="status battery ping mode conv spatial ear mic loud-reduction press tone-on-press rename one-bud-anc auto-anc chime connect disconnect pair unpair list auto-connect volume mute profile codec default latency watch meter tray popup reboot completion debug install uninstall version help"

    if [[ $cword -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "$cmds --help --version --json" -- "$cur") )
        return
    fi

    case "${words[1]}" in
        mode)            COMPREPLY=( $(compgen -W "off anc transparency adaptive" -- "$cur") );;
        conv|ear|loud-reduction|tone-on-press|mute|auto-connect|one-bud-anc)
                         COMPREPLY=( $(compgen -W "on off" -- "$cur") );;
        spatial)         COMPREPLY=( $(compgen -W "off fixed head-tracked" -- "$cur") );;
        mic)             COMPREPLY=( $(compgen -W "auto left right" -- "$cur") );;
        profile)         COMPREPLY=( $(compgen -W "high headset off" -- "$cur") );;
        codec)           COMPREPLY=( $(compgen -W "sbc aac aptx aptx_hd ldac" -- "$cur") );;
        press)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=( $(compgen -W "left right" -- "$cur") )
            else
                COMPREPLY=( $(compgen -W "mode-cycle siri none" -- "$cur") )
            fi
            ;;
        meter)           COMPREPLY=( $(compgen -W "--plain --json --once --interval --device" -- "$cur") );;
        tray)            COMPREPLY=( $(compgen -W "start stop status restart" -- "$cur") );;
        completion)      COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") );;
        debug)           COMPREPLY=( $(compgen -W "emit-case-lid --no-redact" -- "$cur") );;
        install)         COMPREPLY=( $(compgen -W "--yes --no-daemon --no-completion --no-manpages --no-tray --no-popup --with-tray --with-popup" -- "$cur") );;
        uninstall)       COMPREPLY=( $(compgen -W "--yes" -- "$cur") );;
        help)            COMPREPLY=( $(compgen -W "$cmds" -- "$cur") );;
    esac
}
complete -F _pods podctl
