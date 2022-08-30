#!/bin/bash

which curl > /dev/null || exit 1

# This script is a simple wrapper around cURL which lets you make uploads to
# and downloads from a Transpo server.


usage() {
    echo "`tput bold`USAGE:`tput sgr0`
UPLOADING: $0 up [-t DD:HH:MM] [-d <number>] [-p <password>] -h <URL> <files to upload>

-t: Time limit before upload expires. Given as DD:HH:MM or HH:MM or MM
-d: Download limit before upload expires.
-p: Password to protect uploaded files.
-h: URL of Transpo server to which files will be uploaded.

The above options must be given *before* the paths to files to upload


DOWNLOADING: $0 dl [-p <password>] <URL>

-p: Password to access uploaded files

The password must be given *before* the download URL

`tput bold`WARNING:`tput sgr0`
This script does not do any client-side encryption and relies on Transpo's
optional server-side encryption. This makes this script significantly less
secure than using Transpo via its web interface with javascript enabled or
via a client which provides its own encryption.
"
    exit 1
}


upload() {
    while getopts "p:t:d:h:" o; do
        case "$o" in
            p)
                password="$OPTARG"
                ;;
            t)
                IFS=':' read -ra time <<< "$OPTARG"
                ;;
            d)
                max_downloads="$OPTARG"
                ;;
            h)
                host="$OPTARG"
                ;;
            *)
                usage
                ;;
        esac
    done

    if [ -z $time ]; then
        days=0
        hours=0
        minutes=30
    else
        shift; shift

        # time is given as ...DD:HH:MM or HH:MM or MM

        time_rev=
        time_len=${#time[@]}

        for (( i=0; i < $time_len; i++ )); do
            time_rev[$i]=${time[ $(( $time_len - ($i + 1) )) ]}
        done

        days=${time_rev[2]:-0}
        hours=${time_rev[1]:-0}
        minutes=${time_rev[0]:-0}
    fi

    curlcmd="curl -X POST -H 'User-Agent:' -F server-side-processing=on -F days=$days -F hours=$hours -F minutes=$minutes"

    if ! [ -z "$max_downloads" ]; then
        shift; shift
        curlcmd+=" -F enable-max-downloads=on -F max-downloads=$max_downloads"
    fi

    if ! [ -z "$password" ]; then
        shift; shift
        curlcmd+=" -F enable-password=on -F password=\"$password\""
    fi

    if ! [ -z "$host" ]; then
        shift; shift
    fi

    if [[ ${#@} > 1 ]]; then
        curlcmd+=" -F enable-multiple-files=on"
    fi

    for file in "$@"; do
        curlcmd+=" -F files=@$file"
    done

    curlcmd+=" $host/upload"

    echo "$curlcmd"
    response=`eval "$curlcmd"`
    if [ $? = 0 ]; then
        echo
        echo "$host/`eval echo $response`"
        echo
    else
        exit 1
    fi
}


download() {
    while getopts "p:" o; do
        case "$o" in
            p)
                password="$OPTARG"
                ;;
            *)
                usage
                ;;
        esac
    done

    if ! [ -z "$password" ]; then
        shift; shift
    fi


    IFS='#' read -ra parts <<< "$1"
    url=${parts[0]}
    key=${parts[1]}

    curlcmd="curl -X GET -O -J -L $url/dl?key=$key&password=$password"
    echo "$curlcmd"
    eval "$curlcmd"
}


case "$1" in
    up|upload)
        shift
        upload "$@"
        ;;
    dl|download)
        shift
        download "$@"
        ;;
    *)
        usage
        ;;
esac
