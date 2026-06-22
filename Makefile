ssh1:
	eval "$$(ssh-agent -s)"
	
ssh2:
	ssh-add ~/.ssh/leowong121073/id_ed25519

ssh3:
	ssh -T git@github.com

ssh: ssh1 ssh2 ssh3

push:
	git push origin