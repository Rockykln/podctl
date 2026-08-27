#compdef podctl
_pods() {
    local -a cmds
    cmds=(
        'status:full snapshot'
        'battery:battery levels'
        'ping:daemon health'
        'mode:listening mode'
        'conv:conversation awareness'
        'spatial:spatial-audio mode'
        'ear:in-ear detection'
        'mic:microphone selection'
        'loud-reduction:loud-sound reduction'
        'press:stem press action'
        'tone-on-press:acoustic feedback'
        'rename:rename device'
        'connect:bluetooth connect'
        'disconnect:bluetooth disconnect'
        'pair:pair nearby AirPods'
        'unpair:forget device'
        'list:paired AirPods'
        'auto-connect:auto-connect flag'
        'volume:audio volume'
        'mute:mute toggle'
        'profile:audio profile'
        'codec:A2DP codec'
        'default:set default sink'
        'latency:latency offset'
        'one-bud-anc:keep ANC on with one bud'
        'auto-anc:AutoANC strength'
        'chime:system tone volume'
        'watch:live event stream'
        'meter:live dB meter on playback sink'
        'tray:status-bar icon'
        'popup:show the case-open bubble now'
        'reboot:restart podctld + tray + popup'
        'completion:shell completion'
        'debug:diagnostic report'
        'install:install podctl to ~/.local'
        'uninstall:remove installed files'
        'version:CLI + daemon version'
        'help:detailed help'
    )
    if (( CURRENT == 2 )); then
        _describe 'command' cmds
        return
    fi
    case "${words[2]}" in
        mode)       _values 'mode' off anc transparency adaptive ;;
        conv|ear|loud-reduction|tone-on-press|mute|auto-connect|one-bud-anc)
                    _values 'value' on off ;;
        spatial)    _values 'mode' off fixed head-tracked ;;
        mic)        _values 'mic' auto left right ;;
        profile)    _values 'profile' high headset off ;;
        codec)      _values 'codec' sbc aac aptx aptx_hd ldac ;;
        press)
            if (( CURRENT == 3 )); then
                _values 'side' left right
            else
                _values 'action' mode-cycle siri none
            fi
            ;;
        meter)      _values 'flag' --plain --json --once --interval --device ;;
        tray)       _values 'action' start stop status restart ;;
        completion) _values 'shell' bash zsh fish ;;
        debug)      _values 'arg' emit-case-lid --no-redact ;;
        install)    _values 'flag' --yes --no-daemon --no-completion --no-manpages --with-tray --with-popup ;;
        uninstall)  _values 'flag' --yes ;;
        help)       _describe 'topic' cmds ;;
    esac
}
_pods "$@"
