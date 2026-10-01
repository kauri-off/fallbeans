# Fall Beans: a game connection over WebSocket (included by fallbeans.conf). Long-lived, small packets:
# no buffering, no Nagle, an hour without traffic before nginx gives up on it.
proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
proxy_set_header Host $host;
proxy_set_header X-Real-IP $remote_addr;
proxy_read_timeout 3600s;
proxy_send_timeout 3600s;
proxy_buffering off;
tcp_nodelay on;
