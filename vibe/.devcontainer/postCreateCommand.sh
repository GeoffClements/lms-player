sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libpulse-dev \
  libasound2-dev \
  libdbus-1-dev \
  pkg-config \
  libpipewire-0.3-dev \
  libspa-0.2-dev \
  libclang-dev \
  pipewire

cargo install cargo-deb

mkdir -p /tmp/user/1000/
ln -s /run/user/1000/pipewire-0 /tmp/user/1000/pipewire-0

