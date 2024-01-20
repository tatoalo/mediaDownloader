FROM messense/rust-musl-cross:x86_64-musl as builder

WORKDIR /mediaDownloader

COPY mediaDownloader/src/ ./src/
COPY mediaDownloader/Cargo.toml .

RUN cargo build --release

#########################################

FROM alpine:3.19

ENV TZ=Europe/Paris
ENV RUST_LOG=media_downloader=debug,bot=debug,cleaner=debug
ENV YT_DLP_VERSION="2023.11.16-r0"

ARG download_folder_path=/tmp/media_downloaded/
ARG service_folder=mediaDownloader

RUN apk add --update --no-cache curl tzdata yt-dlp=${YT_DLP_VERSION} && \
    rm -rf /var/cache/*

COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/media_downloader /home/${service_folder}/
COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/bot /home/${service_folder}/
COPY --from=builder /${service_folder}/target/x86_64-unknown-linux-musl/release/cleaner /home/${service_folder}/
COPY --chmod=777 entrypoint.sh /home/${service_folder}

COPY --chmod=777 cron.sh /home/${service_folder}
COPY media-downloader-cron /var/spool/cron/crontabs/root

RUN mkdir -p ${download_folder_path}

WORKDIR /home/${service_folder}

CMD [ "./cron.sh" ]