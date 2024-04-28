<div align="center">
  <figure>
    <img 
    src="./assets/logo.png" 
    width="350px">
    <br>
    <figcaption>The downloader is this <b>good boy</b> 🐕</figcaption>
  </figure>

# mediaDownloader

Self-hosted solution to download video sent to Telegram 🤖 (obv in 🦀, lol)

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/F1F7ABOVF)

[![Apache-2.0](https://img.shields.io/github/license/tatoalo/mediaDownloader)](https://github.com/tatoalo/mediaDownloader) [![Docker Image Version](https://img.shields.io/docker/v/tatoalo/media-downloader?sort=semver)][hub] [![Docker Image Size](https://img.shields.io/docker/image-size/tatoalo/media-downloader)][hub]

[hub]: https://hub.docker.com/r/tatoalo/media-downloader/

</div>

mediaDownloader can be used to download and deliver media from a selection of sources.

Sometimes on TikTok (I know 😫) there's some proper quality content that I need to immediately deliver to somebody, this takes care of this in a pretty straightforward way.

There are 3 main binaries:

- `media_downloader`, responsible for downloading the media from the source
- `bot`, responsible for receiving the request and delivering the media to the user
- `cleaner`, responsible for cleaning up the downloaded media after a certain amount of time (externally managed)

### Docker

The container expects to load the [configuration file](#configuration) from `/mediaDownloader/config.toml` so mount a volume accordingly.

```
$ docker run -itd \
             --rm \
             -v /path/to/my/configuration/file:/config/file \
             tatoalo/media-downloader
```

#### Docker Compose

```yaml
version: "3.8"
services:
  mediadownloader:
    image: tatoalo/media-downloader:development
    container_name: mediadownloader
    restart: unless-stopped
    volumes:
      - /path/to/my/configuration/file:/config/file
```

### Configuration

The configuration file allows to have control over multiple aspects of the downloader.

```toml
[telegram]
token = "token"

[redis]
username = "username"
password = "password"
host = "host"
port = 6942
channel = "channel"

[supported_sites]
sites = [
    "site1.com",
    "site2.com",
]

[telemetry]
endpoint = "endpoint"
api_key = "api_key"

[aweme_api]
[...]
```

#### Telegram 🤖

The only parameter required is the `token` of the bot you want to use, for more information refer to the [official documentation](https://core.telegram.org/bots/features#botfather).

#### Redis

The downloader uses `redis` as a message broker and to store the `video ID` in order to save processing/delivery times and bandwidth.
The required parameters are:

- `username`
- `password`
- `host`
- `port`
- `channel`

#### Supported Sites

The downloader uses a `supported_sites` whitelist to determine admissable sources.

#### Aweme_API

TikTok support 😉

#### Telemetry (Optional)

The downloader can be instrumented to send traces via [OpenTelemetry](https://opentelemetry.io/) to a remote endpoint.

#### Custom Scheduling (Cleaner)

The default scheduling mechanism is stored in `media-downloader-cron`, although a custom schedule can be introduced in one of two ways:

- Edit `media-downloader-cron` accordingly and rebuild the image
- Edit the environment variables directly in the `docker run` command or `docker compose` file, namely.
  It is possible to customize the behaviour via the following env variable:
  - `CLEANER_CRON` for when the cleaning mechanism is executed

#### Monitoring

An additional environment variable can be set in order to monitor the health of the `cleaner` cron job:

- `HC_UUID_CLEANER`

Its value must be an `healthchecks`-compatible `uuid` (or `slug`).
I've been a fan of [cronitor](https://cronitor.io/) but [healtchecks](https://healthchecks.io/)'s free offering is more convenient in my opinion.

## License

    Copyright 2024 Alessandro Pogliaghi

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
