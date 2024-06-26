.PHONY: test unit check repl integration experiments

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

test: check repl unit integration verify

experiments:
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_agreement_inv n6f1b2_agreement.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_two_chained_blocks_example n6f1b0_no_proposing_leader.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b1_dead_lock.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_two_chained_blocks_example n6f1b1_two_blocks.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n7f1b0_three_blocks.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n7f1b0.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b3.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b2.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b1.qnt
	cd protocol-spec/experiments && quint verify --max-steps=1 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b0.qnt
