# dnsui deployment (Docker host at 192.0.2.53)

This directory runs the dnsui DNS server and dashboard in one container. The
container serves DNS on port 53 (UDP and TCP) and the web UI and API on port
8080. Caddy on 203.0.113.116 puts the UI at `https://dns.example.test`.

Files:

| File | Purpose |
| --- | --- |
| `Dockerfile` | Three-stage build: frontend, backend, runtime |
| `docker-compose.yml` | Service definition, ports, volumes, healthcheck |
| `dnsui.env.example` | Template for secrets and settings |
| `caddy/dns.example.test.caddy` | Caddy site for the Caddy host |

The build context is the parent directory, `the repository root`. The whole
project tree (`backend/`, `frontend/`, `deploy/`) must be on the Docker host.
Run every command below from the `deploy/` directory.

## 0. Preflight: free port 53

Check whether anything already listens on port 53:

```sh
sudo ss -lntup | grep ':53 '
```

If the output is empty, skip to step 1.

If AdGuard Home or another DNS service runs on this host, stop it now and
write down the exact stop command. You will need it for the rollback.

Ubuntu usually runs `systemd-resolved`, which listens on `127.0.0.53:53`.
That listener blocks binding `0.0.0.0:53`. Disable it:

```sh
sudo systemctl disable --now systemd-resolved
sudo rm /etc/resolv.conf
echo 'nameserver 192.0.2.1' | sudo tee /etc/resolv.conf
```

Replace `192.0.2.1` with your real upstream resolver. This value is an example.

> **Warning.** After these commands, name resolution on the host uses only the
> file you wrote into `/etc/resolv.conf`. If that nameserver is wrong, the host
> loses DNS, and Docker image pulls and package installs fail. Keep a second
> SSH session or console open while you do this, and test with
> `getent hosts example.com` before you close it.

To undo this later, restore systemd-resolved:

```sh
sudo systemctl enable --now systemd-resolved
sudo ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
```

## 1. Build the image

```sh
cd deploy
docker compose build
```

The build needs `config.example.yaml` in this directory. It is copied into the
image as a fallback config.

## 2. Create the env file

```sh
cp dnsui.env.example dnsui.env
chmod 600 dnsui.env
```

No password is needed here. On the first start the server creates the `admin`
account with a random password and prints it once. To use your own password
instead, generate a hash with `docker compose run --rm -it dnsui hash-password`
and set `DNSUI_ADMIN_PASSWORD_HASH` in `dnsui.env`.

## 3. Create the config file

```sh
cp config.example.yaml config.yaml
```

Edit `config.yaml`. Make sure the DNS listener binds `0.0.0.0:53` and the HTTP
listener binds `0.0.0.0:8080`. Without `0.0.0.0`, the container cannot be
reached from the LAN. Set the upstream resolvers and any local records, such
as `nas.test`.

The file lives in this deploy directory and the whole directory is mounted into
the container at `/app/config`. The Config page reads the settings the server is
running with and saves changes back to this file, so `config.yaml` on the host
always reflects the latest saved config.

Saving writes a temp file in this directory and renames it, so the container user
(uid 10001) must be able to write the directory. The container fixes this itself
at startup: if the directory is not writable, it changes the owner of the directory
and `config.yaml` to 10001. Nothing else in the directory is changed. If you
prefer to set it yourself, run `sudo chown 10001:10001 .` and `sudo chown 10001:10001 config.yaml`
here before the first start.

If `config.yaml` does not exist, the server creates it from the built-in default
on first start, so step 3 is optional for a fresh install.

## 4. Start the service

```sh
docker compose up -d
docker compose ps
docker compose logs --tail=100
```

`docker compose ps` should show the container as `running` and, after the
start period, `healthy`. The logs should show the DNS and HTTP listeners
starting. To follow the logs live, run `docker compose logs -f`. Press Ctrl+C
to stop following. The container keeps running.

On the first start, the logs also show the admin login in a box. Copy the
password then, because it is only shown once:

```sh
docker compose logs dnsui | grep -A4 "First start"
```

Log in at `http://192.0.2.53:8080` with that username and password. The
password hash is stored in the `dnsui-data` volume (`admin.json`). Later
restarts do not change it. To reset the admin password, delete the stored hash and restart. The next
start prints a new password:

```sh
docker compose down
docker compose run --rm --entrypoint sh dnsui -c 'rm -f /app/data/admin.json'
docker compose up -d
```

## 5. Test from a LAN host

Run these from a machine on 192.0.2.0/24:

```sh
dig @192.0.2.53 example.com +short
dig @192.0.2.53 nas.test +short
curl -i http://192.0.2.53:8080/api/health
```

Expected results:

- `example.com` returns addresses.
- `nas.test` returns the local record you set in `config.yaml`.
- The health request returns HTTP 200.

If `dig` times out, check that port 53 is free (step 0), that the firewall
allows UDP and TCP 53 from the LAN, and that `config.yaml` binds to `0.0.0.0`.

## 6. Add the Caddy site on 203.0.113.116

Copy the snippet to the Caddy host, then validate and reload Caddy. The paths
below are examples. Use the config directory your Caddy install uses, and make
sure the main Caddyfile imports it.

```sh
scp caddy/dns.example.test.caddy <user>@203.0.113.116:/etc/caddy/conf.d/dns.example.test.caddy
ssh <user>@203.0.113.116 'sudo caddy validate --config /etc/caddy/Caddyfile \
  && sudo systemctl reload caddy'
```

Then open `https://dns.example.test` from a LAN client.

## 7. Cloudflare note

No WARP change is needed. The dashboard is reached as `dns.example.test` through Caddy
on 203.0.113.116, so the WARP or Gateway DNS settings do not need to point at
dnsui.

## 8. Updating

After a code change, rebuild and recreate the container:

```sh
docker compose up -d --build
```

The `dnsui-data` volume keeps its contents across rebuilds.

## 9. Rollback

To return DNS service to AdGuard Home:

```sh
cd deploy
docker compose stop dnsui
```

Then start AdGuard Home on this host again, using the stop command you wrote
down in step 0. If you disabled systemd-resolved, restore it with the commands
in step 0. Then confirm with `dig @192.0.2.53 example.com +short`.

Use `docker compose stop`, not `docker compose down`, if you may want to start
dnsui again without rebuilding. `down` removes the container but keeps the
`dnsui-data` volume.

## Cutover later (plan only, not a step)

This is a plan for later. Do not perform these steps as part of this
deployment.

When dnsui has run reliably on 192.0.2.53, the plan is to move the service to
192.0.2.54 and make that address the LAN DNS server. The likely steps are:

1. Copy the working `config.yaml`, `dnsui.env` and the `dnsui-data` volume to
   the 192.0.2.54 host.
2. Free port 53 on that host, or move to macvlan so the container has its own
   address.
3. Update the Caddy upstream from `192.0.2.53:8080` to the new host.
4. Point the router or DHCP DNS setting at 192.0.2.54.
5. Keep the rollback path from step 9 ready until the cutover is verified.

## Upstream protocol: QUIC (DoQ) and DoH

Upstreams can be `quic://` (DNS-over-QUIC, UDP 853) or `https://` (DNS-over-HTTPS, TCP 443). The importer keeps your NextDNS `quic://` upstreams.

**Before cutover, check outbound UDP 853 from the Docker host:** a blocked UDP 853 shows as query timeouts on quic upstreams.

If it is blocked, change those upstreams in `config.yaml` to DoH, for example:

- `quic://def456.dns.nextdns.io` becomes `https://def456.dns.nextdns.io/dns-query`

Then run `POST /api/reload` (or restart the container).

## Destination map (optional)

The Traffic destination panel needs a MaxMind GeoLite2 country database. MaxMind
does not allow redistribution, so the file is not included:

1. Create a free account at maxmind.com and download `GeoLite2-Country.mmdb`.
2. Copy it into the data volume, then restart:

```sh
docker compose cp GeoLite2-Country.mmdb dnsui:/app/data/GeoLite2-Country.mmdb
docker compose restart dnsui
```

Without the file the panel shows a hint and the rest of the dashboard works. The
map uses only the first address in each answer, and only for queries made after
the database was added. Older rows stay out of the map.

The database is refreshed by MaxMind periodically. Download the new file and copy
it over the old one when you want updated locations.
