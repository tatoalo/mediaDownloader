#!/bin/sh

CLEANER_SCRIPT="cd /home/mediaDownloader/ && ./cleaner"
CLEANER_COMMAND="${CLEANER_SCRIPT}"

start_applications () {
    /home/mediaDownloader/media_downloader & /home/mediaDownloader/bot
}

clean_crontab () {
    crontab -l | grep -v $1 | crontab -
}

activate_crontab () {
    start_applications & crond -f
}

# Checking for HC_UUID_CLEANER
if [[ "${HC_UUID_CLEANER}" ]]; then
    echo "** Capturing ID for monitoring cleaner cron **"
    CLEANER_COMMAND="${CLEANER_COMMAND} && curl -sS --retry 2 -o /dev/null https://hc-ping.com/${HC_UUID_CLEANER}"
fi

# Checking for custom CLEANER_CRON
if [[ "${CLEANER_CRON}" ]]; then
    echo "** Setting custom schedule for cleaner **"
    clean_crontab "cleaner"
    crontab -l | { cat; echo "${CLEANER_CRON} ${CLEANER_COMMAND}"; } | crontab -
else
    echo "No custom cron for the cleaner has been defined, sticking with default scheduling."
fi

activate_crontab