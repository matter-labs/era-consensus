.PHONY: test unit check repl integration

repl:
	cd protocol-spec && echo "init\n step\n step\n step" | quint -r main.qnt::main 

check:
	cd protocol-spec && quint typecheck main.qnt

integration:
	cd protocol-spec/tests && quint test ./tests_n6f1b0.qnt
	cd protocol-spec/tests && quint test ./tests_n6f1b1.qnt
	cd protocol-spec/tests && quint test ./tests_n6f1b2.qnt

unit:
	cd protocol-spec/tests && quint test ./unit_tests.qnt	

test: check repl unit integration
