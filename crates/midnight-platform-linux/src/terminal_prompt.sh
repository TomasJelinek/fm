# The interactive shell is one native positional argument. User paths and account
# names are never inserted into shell source. Interactive rc files run normally
# and can replace this default; a nontrivial exported prompt is preserved.
case "${PS1-}" in
''|'$ '|'# ')
    case "${1##*/}" in
        bash)
            PS1='\u@\h:\w\$ '
            export PS1
            ;;
        zsh)
            PS1='%n@%m:%~%# '
            export PS1
            ;;
        sh|dash|ash|ksh|ksh93|mksh|yash)
            _me_prompt_uid=$(id -u)
            _me_prompt_user=$(id -un 2>/dev/null) || _me_prompt_user=$_me_prompt_uid
            _me_prompt_host=$(uname -n)
            _me_prompt_host=${_me_prompt_host%%.*}
            if [ "$_me_prompt_uid" = 0 ]; then
                _me_prompt_mark='#'
            else
                _me_prompt_mark='$'
            fi
            PS1='${_me_prompt_user}@${_me_prompt_host}:${PWD}${_me_prompt_mark} '
            export PS1 _me_prompt_user _me_prompt_host _me_prompt_mark
            ;;
    esac
    ;;
esac
exec "$1" -i
