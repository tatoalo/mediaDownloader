FROM clux/muslrust:nightly AS chef
USER root
RUN cargo install cargo-chef
WORKDIR /mediaDownloader

FROM chef AS planner
COPY mediaDownloader/ .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
COPY --from=planner /mediaDownloader/recipe.json recipe.json
RUN cargo +nightly chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

COPY mediaDownloader/ .
RUN cargo build --release --target x86_64-unknown-linux-musl

#########################################
FROM alpine:3.19

ENV TZ=Europe/Paris

ENV YT_DLP_VERSION="2023.11.16-r0"

ARG download_folder_path=/tmp/media_downloaded/
ARG service_folder=mediaDownloader

RUN apk add --update --no-cache \
    curl tzdata \
    yt-dlp=${YT_DLP_VERSION} && \
    rm -rf /var/cache/*

COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/media_downloader /home/${service_folder}/
COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/bot /home/${service_folder}/
COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/cleaner /home/${service_folder}/

COPY --chmod=777 cron.sh /home/${service_folder}
COPY media-downloader-cron /var/spool/cron/crontabs/root

RUN mkdir -p ${download_folder_path}

WORKDIR /home/${service_folder}

CMD [ "./cron.sh" ]