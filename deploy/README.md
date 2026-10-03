# SlavicFall host deployment

The host uses Git checkouts at `/opt/rustyfusion` (Ambfibia/RustyFusion) and
`/opt/ofapi` (OpenFusionProject/ofapi). Update from a workstation with:

```sh
ssh user1@api.slavicfall.ru 'bash /opt/rustyfusion/deploy/update-host.sh'
```

The script pulls fast-forward commits, updates TableData submodules, builds the
server and rebuilds ofapi when its commit changes. It snapshots the database,
installs these versioned systemd definitions and restarts both services. It
refuses tracked local modifications and overlapping updates. No copying binaries
or source files from a workstation is needed.

`config.toml`, database files, `/opt/ofapi/secret` and backups are excluded from
Git. The live server config disables the TUI, binds login/shard on ports 23000 and
23001, advertises `176.123.166.58:23001`, enables the monitor at
`127.0.0.1:8003`, and uses `database.db`. Public authentication uses ofapi cookies;
plaintext password login is disabled. ofapi uses `/opt/rustyfusion/database.db`.

Nginx and service templates are tracked here. To reapply nginx configuration,
install `deploy/api.slavicfall.ru.nginx.conf` into
`/etc/nginx/sites-available/api.slavicfall.ru`, run `sudo nginx -t`, then reload
nginx. The certificate is maintained by Certbot. `/builds/` returns 404 on both
HTTP and HTTPS; old game build archives and OpenFusion were removed after the
database migration. Upstream `statics.csv` stays unchanged for clean Git pulls;
its missing builds directory is never recreated by this update script.
