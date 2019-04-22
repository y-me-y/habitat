HAB_SUP_TEST_NETWORK_SIZE ?= 3
HAB_SUP_TEST_NETWORK_PEER_NAME ?= peers_1

up: CTL_SECRET
	@docker build -t hab-sup-network .
	@docker-compose up --scale peers=${HAB_SUP_TEST_NETWORK_SIZE}

down:
	@docker-compose down
	@rm -f CTL_SECRET

start:
	@docker-compose start

stop:
	@docker-compose stop

console:
	@docker exec -it $$(docker ps -qf name=${HAB_SUP_TEST_NETWORK_PEER_NAME}) /bin/bash

# Kill the specified container to simulate a departure
kill:
	@docker stop $$(docker ps -qf name=${HAB_SUP_TEST_NETWORK_PEER_NAME})

# This gets the IP address of one of the containers that you specify. This is
# useful if you need to e.g. hab svc load --remote-sup=X.X.X.X core/redis
ip:
	@docker network inspect $$(docker network ls -qf name=supnetwork) | jq ".[].Containers | to_entries | map(select(.value.Name == \"supnetwork_${HAB_SUP_TEST_NETWORK_PEER_NAME}\")) | .[0].value.IPv4Address" | tr -d '"' | cut -d '/' -f 1

CTL_SECRET:
	@hab sup secret generate > CTL_SECRET
