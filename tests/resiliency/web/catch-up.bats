GIT_ROOT="$BATS_TEST_DIRNAME/../../../"
MAKEFILE="Makefile.dweb"

load '../../test_helper/load'
load '../../test_helper/web'

function setup() {
    load "test_helper/bats-assert/load"
    load "test_helper/bats-support/load"

    mkdir "$BATS_TEST_ROOTDIR"

    (
      cd "$GIT_ROOT/docker/" || exit
      make -f $MAKEFILE clean
      for i in {0..2}
      do
          make -f $MAKEFILE start-single-node-detached-${i} || {
            echo '# Could not start nodes...' >&3
            exit 1
          }
      done
    ) > /dev/null

    xxd -p -r << EOF > test_dweb.zip
504b03040a0300000000af680857dbff951917000000170000000a000000
696e6465782e68746d6c3c68313e48656c6c6f20466f6f626172213c2f68
313e0a504b01023f030a0300000000af680857dbff951917000000170000
000a0024000000000000002080a48100000000696e6465782e68746d6c0a
00200000000000010018000029f7881acad9010029f7881acad9010029f7
881acad901504b050600000000010001005c0000003f0000000000
EOF

    # Give time to the servers to start.
    wait_for_server 8000 8001 8002
}

function teardown() {
    (
      cd "$GIT_ROOT/docker/" || exit 1
      make -f $MAKEFILE stop-nodes
    ) 2> /dev/null

    # Fix for BATS verbose run/test output gathering
    cd "$GIT_ROOT/tests/resiliency/web" || exit 1
}

@test "$SUITE: Node can catch up" {
    call_web --pem=1 --port=8000 deploy test_dweb test_dweb.zip
    assert_output --partial 'https://test_dweb-'$(identity 1)'.ghostcloud.org'
    check_consistency --pem=1 --value=test_dweb 8000 8001 8002

    call_web --pem=1 --port=8001 deploy test_dweb2 test_dweb.zip
    assert_output --partial 'https://test_dweb2-'$(identity 1)'.ghostcloud.org'
    check_consistency --pem=1 --value=test_dweb 8000 8001 8002
    check_consistency --pem=1 --value=test_dweb2 8000 8001 8002

    call_web --pem=1 --port=8002 deploy test_dweb3 test_dweb.zip
    assert_output --partial 'https://test_dweb3-'$(identity 1)'.ghostcloud.org'
    check_consistency --pem=1 --value=test_dweb 8000 8001 8002
    check_consistency --pem=1 --value=test_dweb2 8000 8001 8002
    check_consistency --pem=1 --value=test_dweb3 8000 8001 8002

    cd "$GIT_ROOT/docker/" || exit 1

    sleep 300

    # At this point, start the 4th node and check it can catch up
    make -f $MAKEFILE start-single-node-detached-3 || {
      echo '# Could not start nodes...' >&3
      exit 1
    }

    # Give the 4th node some time to boot
    wait_for_server 8003

    sleep 12  # Three consensus round.
    check_consistency --pem=1 --value=test_dweb 8000 8001 8002 8003
    check_consistency --pem=1 --value=test_dweb2 8000 8001 8002 8003
    check_consistency --pem=1 --value=test_dweb3 8000 8001 8002 8003
}
