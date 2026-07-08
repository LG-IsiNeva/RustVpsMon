# fail2ban: ban IPs on nginx 4XX

Bans hosts generating too many 4XX responses in nginx access log (scan/brute-force noise).

## Install (host running nginx, not the container)

```bash
sudo cp filter.d/nginx-4xx.conf /etc/fail2ban/filter.d/nginx-4xx.conf
sudo cp jail.d/nginx-4xx.conf /etc/fail2ban/jail.d/nginx-4xx.conf
sudo systemctl restart fail2ban
```

## Defaults (tune in jail.d/nginx-4xx.conf)

- `maxretry = 20` 4xx hits within `findtime = 60`s triggers ban
- `bantime = 3600`s (1h)
- `logpath` assumes default `/var/log/nginx/access.log` — adjust if nginx logs elsewhere (e.g. per-vhost logs)

## Verify

```bash
sudo fail2ban-client status nginx-4xx
sudo fail2ban-client set nginx-4xx banip <test-ip>   # manual test
```
