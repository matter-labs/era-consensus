use crate::chonky_bft::{testonly::UTHarness, timeout};
use assert_matches::assert_matches;
use rand::{seq::SliceRandom,Rng};
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;
use zksync_protobuf::serde::Deserialize;

const BEFORE : &str = r#"
{
  "msgs": [
    {
      "view": {
        "number": "68",
        "genesis": {
          "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
        }
      },
      "highVote": {
        "view": {
          "number": "24",
          "genesis": {
            "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
          }
        },
        "proposal": {
          "number": "99",
          "payload": {
            "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
          }
        }
      },
      "highQc": {
        "msg": {
          "view": {
            "number": "24",
            "genesis": {
              "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
            }
          },
          "proposal": {
            "number": "99",
            "payload": {
              "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
            }
          }
        },
        "signers": {
          "size": "100",
          "bytes": "1b/+k397++////9q4A=="
        },
        "sig": {
          "bn254": "oBd0ynWdCVGgVYy59SjxyNTuGLgbmFmjB7sHiIQZWSfZai/GqONWvjhfx4BqHE9x"
        }
      }
    },
    {
      "view": {
        "number": "68",
        "genesis": {
          "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
        }
      },
      "highVote": {
        "view": {
          "number": "25",
          "genesis": {
            "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
          }
        },
        "proposal": {
          "number": "100",
          "payload": {
            "keccak256": "pr5OnNRgXJIsOaAGeTHcSFONmgDQn7kQ4nwMHbzmA6I="
          }
        }
      },
      "highQc": {
        "msg": {
          "view": {
            "number": "24",
            "genesis": {
              "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
            }
          },
          "proposal": {
            "number": "99",
            "payload": {
              "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
            }
          }
        },
        "signers": {
          "size": "100",
          "bytes": "1b/+k397++////9q4A=="
        },
        "sig": {
          "bn254": "oBd0ynWdCVGgVYy59SjxyNTuGLgbmFmjB7sHiIQZWSfZai/GqONWvjhfx4BqHE9x"
        }
      }
    }
  ],
  "signers": [
    {
      "size": "100",
      "bytes": "gBAKgTQASGKACIIAgA=="
    },
    {
      "size": "100",
      "bytes": "AAAAAEAAAAAgAAAAAA=="
    }
  ],
  "sig": {
    "bn254": "itXeKl8A7Gw1YLm5ijRo0epof7myR9oL6LNZSP/T0fYsn5iC4KVxUvEzstWIUWd6"
  },
  "view": {
    "number": "68",
    "genesis": {
      "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
    }
  }
}"#;

const AFTER : &str = r#"
{
  "msgs": [
    {
      "view": {
        "number": "68",
        "genesis": {
          "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
        }
      },
      "highVote": {
        "view": {
          "number": "24",
          "genesis": {
            "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
          }
        },
        "proposal": {
          "number": "99",
          "payload": {
            "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
          }
        }
      },
      "highQc": {
        "msg": {
          "view": {
            "number": "24",
            "genesis": {
              "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
            }
          },
          "proposal": {
            "number": "99",
            "payload": {
              "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
            }
          }
        },
        "signers": {
          "size": "100",
          "bytes": "1b/+k397++////9q4A=="
        },
        "sig": {
          "bn254": "oBd0ynWdCVGgVYy59SjxyNTuGLgbmFmjB7sHiIQZWSfZai/GqONWvjhfx4BqHE9x"
        }
      }
    },
    {
      "view": {
        "number": "68",
        "genesis": {
          "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
        }
      },
      "highVote": {
        "view": {
          "number": "25",
          "genesis": {
            "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
          }
        },
        "proposal": {
          "number": "100",
          "payload": {
            "keccak256": "pr5OnNRgXJIsOaAGeTHcSFONmgDQn7kQ4nwMHbzmA6I="
          }
        }
      },
      "highQc": {
        "msg": {
          "view": {
            "number": "24",
            "genesis": {
              "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
            }
          },
          "proposal": {
            "number": "99",
            "payload": {
              "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
            }
          }
        },
        "signers": {
          "size": "100",
          "bytes": "1b/+k397++////9q4A=="
        },
        "sig": {
          "bn254": "oBd0ynWdCVGgVYy59SjxyNTuGLgbmFmjB7sHiIQZWSfZai/GqONWvjhfx4BqHE9x"
        }
      }
    }
  ],
  "signers": [
    {
      "size": "100",
      "bytes": "gBAKgTQASGKACIIAgA=="
    },
    {
      "size": "100",
      "bytes": "AAAAAEAAAABgAAAAAA=="
    }
  ],
  "sig": {
    "bn254": "i4VALQ3T+rCadiSKXvDSfhXB0aKAWsCkmreCPiUH8h8dRCHCDQENyc9lP6AcnAmQ"
  },
  "view": {
    "number": "68",
    "genesis": {
      "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
    }
  }
}
"#;

const COMMIT : &str = r#"
{
  "msg": {
    "consensus": {
      "replicaTimeout": {
        "view": {
          "number": "68",
          "genesis": {
            "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
          }
        },
        "highVote": {
          "view": {
            "number": "25",
            "genesis": {
              "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
            }
          },
          "proposal": {
            "number": "100",
            "payload": {
              "keccak256": "pr5OnNRgXJIsOaAGeTHcSFONmgDQn7kQ4nwMHbzmA6I="
            }
          }
        },
        "highQc": {
          "msg": {
            "view": {
              "number": "24",
              "genesis": {
                "keccak256": "M1NavRkNSOPEByFj+16n7cNr02eKa4hgVUDommmxIks="
              }
            },
            "proposal": {
              "number": "99",
              "payload": {
                "keccak256": "i7P/o32AiegiItCUzBy+PAhnxxrterQx1ipBgxiz/wg="
              }
            }
          },
          "signers": {
            "size": "100",
            "bytes": "1b/+k387/+////9q4A=="
          },
          "sig": {
            "bn254": "gsg+KZtJ+E8OgZtDjmcEjNVKUUBHfvKHI9GpEmvm5cWbGastKxhmRsmgEJpqSOoI"
          }
        }
      }
    }
  },
  "key": {
    "bn254": "qFDTWFSGM3szB0VuqUqp6mJNrTAIGzXfwc0PQPYsbG/WDBFtTR8BH142rkn/IdudBKHizc3pbxF56WyL6qbq8CZgXdeqawkyiyqwqqHTHPpgt0GptwLduNQYIclSv04I"
  },
  "sig": {
    "bn254": "qJSNn3yD2x8zQcPshAaSevdf1ZHY/CiDMP0U4ek/lMkm76x5d2UZ3yKTMlJ8b+Mf"
  }
}"#;

const GENESIS : &str = r#"
{
  "validatorsV1": [
    {
      "key": {
        "bn254": "gMCw0lH+sgsHJHgU95HcAGNtcdsGazwS8VgynzEjnNMtHs0OFslevNkI1fadWQ5eFGYLG+ZKCewYLKo8qrQFATuLJuwgu4mxTUIwh8pbLwtL9X6HI5Abnicjai3RxZBX"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "gbv/EA7KJyrDcJ57N1zefw2bZf6I4nDXJiN3UqXUJtPsGSjVqaOlhTlYED3O5UlMChyBntFQ4L15I3AJ8jM6ygmZLm2F2KRQ57TOd4QkadxohRD4xc13Wt8Eno00KWxp"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "gpR3INceHepI4nc8u7cpna7MSZEr/pYIF3RQpA7J/AeeHx7iFg0ue2QVqZmX6qKzASfYikx/Er4BaTjE49UIuVDJ6DlNDQmNDTaviTrL/p59JgGbfYNhmwg/RQLl16Rv"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "gxjnRgI9MsWPgt2yRNcOKboZMXI5fDiXe3qtnvWTc3XA02Z84DVRXGRtATkO894pC1zkTL/QdhrVnxq7HRvcuixKu0yNE3m9baltLXKaHNwNa9YfM4f7rKgpdo57ptSN"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "gybB3lVFLrqpJDtFlUC8OnvV8VsiFj8ve47gN5G9gPDM4CmjaLe8UAKjXAIvqJB6GTKKHd+EfV5r6F2/2VW11Jx8mYW0fkRsU/s1LuzSBQMtHoX1p5LAlDInXDvr97uj"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "hFxom2kJdftdfBe4NXpyntwWaHJakVdARSy/h3+MWUu1w4/f19ey5iWAjM6voZQWF+sP5ZgifBI9kubtqREGBfb7yCuku0k4mtDqeho4CU/4CaOr0ZdC3pyMsO474S5u"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "hHbswDN88aAgszMgOO8pEiJl3MUHGy75CHFz39AgSzB5/a0EJtmPboG9m9SuPsQ5EkWVAWness96nK9Fasx0GeFvVqHavn8/AIPz57JhVhG2tFTF2QrkSbId1r6GFGVV"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "hLZzm7382AhXvcjBQYkUdlmrPB3JR0TkFB2vZ2xvPsB1mZ/mX9x8vo5+adW/gSEhBYaMazH/jZHE7HGb/X0nPAzvyWmWsVRhmSvyv9ZqFRXZhSirdl5InltinOCJrpqz"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "hUezheGDqx9yZNry2UiMDcrWyqli9du+yRbhL3Jqn0uSI+pbc2Q5BBMHeMzWnS/AD3Pr32tOje7PONFMDbW1MxUUQYe8/aXFyLnWgF+55d6am16xmLXdo/b0EUg8ArAw"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "hdSzvoBI0yXQcvnWo9MNJnhH2zFUwV1xfi7SJoRkJ+RT11eHNGmQgym+cmNI563wFOmae+qTF/lulq6tzjyNgf0wpDUMUDwHqI7ebZ46HGsKayhTuwqBbBklptaSO49A"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "h/CF2GTPvEKgagS1PXbhN0ASQJp6skQt975XEtbxXU6l7GmYjHVMODvB2WAMAZh2DMq24j1ByXgRRlpyNWsYbpHZ5eA3ROlcz8tSlcJgxg7612F07tzxMNtsQGUTFrqB"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iAc6HHC7/7eQV5UbyanoVk3Mcv1ToWAwtPZGOFeheQFh3Byu7d0bSlP9nPz/ytKqFzeY85nmhmiUBkVZs5UMxPtGwazsuHEnriTdK43pJfHvwvYNy0S0oLVqJ83EFZh3"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iBIDL1KcadsAesoe6f0cVJEMn+Xn4ys3DBEC9fFw3mFUIdSoXI8O82sLEmUB4uUDBKqRPFIDuCNFxll6Ic2DT3IP19n4uZkJ+VI8k/teQjaMVgIJTPxNh58J0b6mHom8"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iDmyFU4siJBrW39JnDswo7Amvc5vdUQi3LEw5bjOtya52hxx47+1Wlqs23B6q/BMA0fazIAGjrElTlk1a48tmV5oltS8ypTsktf1KkBD4T2PfM7mIawWem0pfA3BRh2j"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iFzl2kBHM/Ji0l4334lGv4G4/5Ul5ChCmv6lpH/8EGbjx8rBVYMk+r6Tl36qkV6vAKe24qIwDF7hoWifXfdgAPTErPn3sDhQSiU3gLsYDWonfrRpFgjzIdUiQ7GpkgyS"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iH8fj6rbxyVrgWAMkPfMb6oIchW7Hk8HfOFD4frxZzNGguxkqlR8u5XaUzWb25i5DUE3AbFcfPgKYcQKhwUug3c7ITrO7eVuqhXmq2FTNaAXqKR9/9vZJaK59vVBVU9G"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iQJTWSxsrqHK6Eo36WrAlvHDmY0DFbl6UkvYosq2UcoFeNdReGhJSsNSG928SZ3EEB0ixUxOz9cTpTbzV7XuPRoBVLZ30K47/3e1fWt3Rzl0m0td3uBvrfZPWuEXXsUo"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iYeJ52dD/z3zMonEKwOgVi56O5jZ0x0+Xlqmt19Uif2j97c06PHNT9tKR6RO2HbYGcz8AYENxMi+UvqoforZsTA/Gnv+PrU0RP2Glehi/OXjI7dPLNOlFpW6NosxZqJr"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ihqMoKBUnlLa5TdDwZKzbeItXdhVVA8cRvhvdZ7dStY5mJ5q5F/DqrTvrnN4cVpyE19ny/Lorv77nTQaTVRI7ZwI/SX2p0ehCELfKnQyM6CFVSV7ZDgtoKv8sMsd3CmG"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ikvr3Eb/KKrnrhmC6wGCe/F5UvPxGVP7GPGWqKzYYpywMMmGRwhC23knu5Lb2M1sEsygJiU1u6soCPjm0EG3yrayoaImuMoY9ydgGVMYlQYDgJ5g1pcMXvc6Df2sP0nb"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "iuluXOxTROKMr9yaJt4gq/y6FI0THFglCJVMPcEOh9SOy2ZSXDI6ZTmykYyv21KaFYADiRs8EBgzpqcCXn8rPE+MoyuoeiUBJJiJwuadAVVhfKOe610wseiYG1CfvGHs"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "i3x7VNqEBF1bZPyN1qjqHf9ETpXfGpIGbtPcCn+zqb+wxMyfOpRg/jjp2uMxSBkNElOhXRbb331u7+WLWpesp92HmekQRWBkX+GVSUQ1N8gAlRzOAa9x01dy6wqvs31h"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "i+xGKSCcztl8P6Kg63e0YXqZyoycBvyRFROC3bd2abvIgOtfe9kN0L4D5HNtwKfRE5NoySBDF7oEsDRLp84CY8mg/6ZNRNL+5zAS5w5z347Nf0iw2drKVAsuZ4V9JsMo"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "i/ULswKmO1oPC5GmcsWlfPKdoMYiBriu0O/PzmIskWxTA9sp/C6sL6ON41ZMp31/ClenKW1lcnlvG1pHLZX3ZBYKY3KMNDOmmp69nu+jd9lUqoLG8XYG3AHrdqmkRdI1"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "i/v+sVaDQOnG8/rV+A9CC4ulY/4OgjzdYvgpxzzhULSrFP5haFrr8LvjIpNlm+fbEG4tSgdQlgw3Nfhp0Rc0VNbX6vCq1M8SDMhET6xKv43rgXRnKRGoW6ujI5jHA2ZV"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "jROmFhkRBR6zPuX2mPp1Ir/blaxoquxSeRFwO7WVyjydq14OI+ntmGxt3wylw+vPC4axY+sxoadSfpnYvIM5lHwr+Nz0RtRli9PL/ATyRnTj9GKzmd73NAgaFEeyAdlJ"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "jUzGCW/YpT7jDhXSdZ7Ie9EC6tRyJF8Fok/gfeyxOIFijfm0AFroSiiXdQsP9owDFUg/w//p2IN9IvndFKVc7iFtZtOAfTdKz6ALhGuT9vpmm+Cal3Gq1zAfUW7OoOwi"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "jl3kujxCT+F4Galnb6LiZLytRLAUHfX/2O7M+U6fK+6S6kHdVgYdc3pnofj2z6LPC/srBU/AuIkB4OaFHdwFQvcnk9M33+ECAWaZiNdyjvGzGV8vNWHcl5dFVUWz0Iy8"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "j1SEO3GWCaPIWBVAXimGLwFBKW80cAfPrxWrW5X3KrA7lbahpj84ZMW7PElPCKGXCUbPpI9E3RFCkBRYhbjkDgxIvadkGM7deYPlilRsFmQDTYRFhmSWalNBXGL/zMCt"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "j5/xy8CHCieMK29pkg8tCjBvuuGSRB2X0hjEPenjsB2frHJEN5a8T0z/xzgc3RLAE2IB4b0uAJaTN++PUAp62pqpSwdyS28LvBOm9VglIzEVtLeYw8pLv49Oop0hST0R"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "j/75Z9c7WHLM2UIe5BRLFr8Q1WWINLFeOyGvhswKR3m2jvCKOlewpgqx7m3X9iA0E2H+MFBg7rXwJWW7MlSJc8A9HroRyp9FKvgkQ+3uvrkbnXgpoPJz10Lz45w6HbdE"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "kXbQJpaRcXytFHuFA6b+Y9bdhWPSopCyMnNm9pgbLSkD86Ij8YGbwqBWHdmdTW6kCxjh9mmpxbFLZEyBB9dV0gnPsmNaOTNUwBZo7wxY8TzCSy2PrJl27tirAAS6S80T"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "kk3ULhMa0+sT4mDoiNpT56mFgS6Hne76YF2Yc3pr0rzVwH0FebjNg0mH78F1mhvgFUEapWaNYnctzSJdRBFPOqSxXhmHA8vlESEv/lEJ/wbUx6nvkNMbCaCwZTPzJAnR"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "kvgZ5z3/tHpOfDGFwWN1Lj1YuYOkJqBc7gjt/dd7U0LHRBb+g6djpWEbNZOf6av7ANi9RakljUpkugR5wWxz38yZ6DjaWKdKuG0wbxxnKsuGNa55yLfGT0YunIyco3Ub"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "lAyNeoz5fj0sNnXExs7bcGVHt6vMqOeXxew+CiuuO45SLcxHUt+XjYaeqW0lcj0QA6V+4+GWDE+PaGSCN9eHJeu72Tz5Hju8C43a+KfW8W8rK8tyarkVu0bYxkV7cHSo"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "lIP8YY8Rp4R8L/9C5kWcc8oUOZpV0VzTW5u/t9fobjzsrsMtHP12Aq8BqnxvkjeKBdoD4WQTLMovuVLMS9WX1cP9xxbBT4wmF3BVbNyKBlU03lx6t+U7rFIzCwGK9pBx"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "lTE03rZOuFa/DBdm4EjSJUGXBOIMfBEJ9Clypmiqx8GcRCrzaTJt9LFa9Kel3fNGCFu0o6Mik33fm1eXSZ3q3FEDWdil4fOYcjpQGTplpqnzmvYOxIsbppHAR4Hhz5nh"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "lV0pbA0HrSYFTd7vWPFoExdkh958xziPTTV0GaGtSyXbcEynOPWjZtzjySCfX3UiCavVsBQ5u6oJOgFUEhcINjQ3AFDaJlBXiTDalKVF1WL4A205HibFAx60gltS+h19"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ldDoT7voDma5N1/bR6+xjGJv+uQPWDPiGCgOZtGTHIXl/AkiiZm636Tov5R06tuXBh4Em7iV1BePdAD5q2PwKh1mQVCPQjNlhWJvwt8BHYD6Cvc6ZDANAQ4AT8+vCSHu"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "le+0wEFdHva/Nzsv49KXHlPRSCe7DirAoiapPpSo8zPaT0OPfWOAQXD3gBlLW8LREzGcBX1W8CqWGMv/xnyzFMsIIZfzTfQYleKKAn/3bY7+vtcfWoyMFCN7pOw68txK"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "lyZcQMdToQaRT2FTEUH+PT+shl1G5/F9HDXprKTgkYmwg1YwYwBnAT+T7eTdAMj0ApdaIaJ6zdoXVXL8Nj5xiVRWLXpqUgucATu1qCHjiKxhybVqE0neu2/3EjVhKTYe"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "l5gjy7UxGyMX5Ru9ZwbrmUenX7HAfB4VMob3BFO3yMWm+4RXVXNG5xOgHPgw6mfNA57C+DaXPawOmkCENgbHF/N6CuHqdX9o1v9mhNEWK0WyCiO8HUPTDngNEQAiqUAE"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "l7ZsVMAVKz9FuizdldgcrUwSoH7qEGgavw5Y4Nwx9gGTNsN64tOTtwfQAlyvOFfNDMyAXp5nrZx7MRrjf32eTCLragpcjG3g/5zX9co2Qu6AIzu1b6Kzvva6efXrLNPY"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mDVCxOCVLJ2u9oi7DTtP6ZV3+XaepOaOQ/c2ld7SmmJNJr8wvTaZpiGpz0vYiuskCLDCvJpbv5r735fgABznXdysRIL6BaFx8bGVe6nTi7IBg0YC2TDHjTK9eyBsRcRr"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mJ7ueq3ANzFX1T8Dcc0jyhfoDndniCnVwidMXl7XYbihck/0IdjRl6Z3rwVDWBjPFtcO2u2Lq8E7TQcjkCvbJU+n2WuToY4VI2dZLEbZ3RD4LFPflnrToAWzVOPy+Zlh"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mPHJ0JEuT74AzUaIp9dAeNplx4U5EbOPGXfRhQ1mKVAW3yspkCSR52CM/hecLjq1EwuSI3LO8eiFV8R080sFBqF6azQQhNCAlo29QTlkjp1hFgOVegTtnSVpcwO6wv7C"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mPVYC5MVxjryobMTAVdU3m9oDfglgHJL/MxFQUCNPKnlvsghmzrq7M6jl+SePM/LEsO1WlWvCtD+6xIFJFdNmwhx/jlP5W5R56D0jc6AMHkE2UIOd0y76iPYpqN0QP+B"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mSwT7BIGJza8SyJNkd/lABnZCujorip0J4iexGmaFC1c4x1/l2zR3XWwk69mqlRRFl6CSVSrSIksrPsPuPvw2pLajg3Hw2G61UB7UIf+OCtxBoGUAoE+b1p46WqJp3a9"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "mdfmEa+Df1gYHTIlCP8gTQwHNJWSUvmguhfzT02tUnvWBzNhlAT2WGBqflgb2J4kF16869zaE+yXhVWkXyC5rQ76QQ3ZmtnIgJQ0AFlYHYGYd/wpKkWcQpDVSHjTLIJZ"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "oKUiBdjCPbic8HIdcRB/ng0Ty2c/RIoQCP6WunzrwWSI0XfAbBgjeTiZ2lWGWSxtATJ4mR2HBbE1KQheKw2/FtkghYa1SxXFAkdcRz2mv0pr1td+nHc7TzaZgI96/GNO"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "oqqYUBBWIcg7PbfasXoFyuErs6t0voOc8ghqzypAqGMFKpfKH7DRPmADo9MLDzTaAs9LBkM7v5BKRZRWWAXfshhkg6C7z5QK5ZhGgjHbfcFjirGLSnuh4wmdPGMEms2/"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "oss2meq4yeV7F9r061ibyh+tTpkKK0A3jRSHorpf9tA8dQ21wX9GYVbw3zDIdyrXCDSObGFA+md6cQY9blv6TL+yQwHJkd4Pfsqh8u5g4dNl9Ng2sy9dsEZ5p8KUO+qY"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ovRNmY8Zl7cu7CvyUtMoqezpoWticRhMX2gsaXSjpPE3qGn6Ghi7L04senhc0cpCErP/CITdm1HM1uw3prw6fMLUq4GsV2cTwZaVkCFtaSUVM2IYkgX/PEQIHCc4ONkw"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ozwAqD3kVyO0BxfE8ZXsV8SqVLzybYx1nNlxpWkaIkTGw9/TXX/IpkgAtoNRWT7WAXlnbPSJ+YAHgFYavWbgY9AKh59aG+i4k8JRoxdc8DtiyUmDkb4HHmivNhu80L+U"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "o01Bl5ynSTzvwglK92FRM2qDnj8UukU2U+BeDyi278gQtsy1U2PRI5J9nX9u6veOAeUAL0O6TR5s9GHbNAFV5Kk5b198ZNf6kVrIivTsCjhZtST6bUsF3QCXmbZQTJdd"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "o1gMn1BSQS2i7TI0RdZN5S9LRmUslaabgw2tNROLwGBmBTyK1fk5z8XNkqopq3pLE5d35x4jWx+HAhJm0+m1hRCuwkYK4v3j7pqYdoUENMOr2jkE63KuDQfN0KbMK4wC"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "o4tnUGhbZZ6Wf7WfzMZ1fiUbI70SNwhcF4bVJRtiwg7fmv60VlA81KkmKTKTjRZpCRfDEtDgACbaMVDM8UOmxRET2CBvEa0/YLlsdI6CqzrVZk05ojR/yy8A/CcUWs7g"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pGDxcz8e8IMu2qJx9b+bGya+cv7DBG8gjU4FHRLAuO1pNHXeC7sFscfJhK/msbyIAYSsLnGngRwd6v1Y+C49DcqkFWJE6i9bE/R3Sraxn24UDanOEEtLMO6vsdxH2hm2"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pIyByTULhUOO5VlBvjH1RLQp7teWqaBIB3h4T6QJ8ptzkoG6VhhGNNYOsuTYiZgRE8L7Kn9l+s39B44BT61J/iGO3xIMoSe4/AD2ypc84qVpWRnh7S4Uo5/qYVHJlwCn"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pQ9VTxo6RJXKGX/p6Z3402UgCx7BRkJmZAcoRzbFEoj1qq6tt8wg56NLEuwn+mtgAYifPPnVWqshKvmPwEIjsr3t1eyngDemNj7MkjPPzSuGGXWPoM2NsCxDC5SrW5uT"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pVfrhK+aOMzEBRl/nOPbJv2DXa8008pCx2nfif2xAvxmKydAu4ye9kcqNe/EWFAdEkH2LUcaUrCicB99pIbhA5FRY7FQVcVtL2FvpVK/4faHctFmpT2b/b5ugMr1QRGX"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pixeLjb+dzedkKw6JUGhT1FkbjENGCvek7QX2xYCXtBFbMji8aN/xrY+iwxAnGQNF2bdCPFFcGK0AhONHwxhx9eOTL9rwGkLzQ/mKD69u6xKe9iphe4oxOE+WDAlfJYd"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pi/Wk+lwWn2HhhL10uxcAP5tmU4d3pX2eycDxBOmGv/hl7zcBbW9SxjWDB+2wK4yBjT1jD8wq19hknMEZqIDwmNW+YmMlCBn4klsLxUzM97vT5xjF3kbVdm4BJM5ZQG6"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "pngu6eyzf4+vPcNV8oqTzGhmKnzpwSF0cYSX73yN11mFoDKJvRIOj9zSQMPwpq7PFq5sr8ofZJSmw6TFYxkodKkbmEUpn1bac5LwtGi3XXp/aWoiZlaTWsGFe4OYcxzi"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "ppc7l5ARgeRBiKt9p5Y55uROvdCSmtaq5jjeEm27tk9QTC1pfnQddVtlBWnomQ8dCeXK3cWAWVe8bU4zjP35ltYIvbIDootl4e3ZvmLbmR7Ec0wbFcgu4oMfjbdWthOq"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "qFDTWFSGM3szB0VuqUqp6mJNrTAIGzXfwc0PQPYsbG/WDBFtTR8BH142rkn/IdudBKHizc3pbxF56WyL6qbq8CZgXdeqawkyiyqwqqHTHPpgt0GptwLduNQYIclSv04I"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "qP8oEwE1wZiD9Ibc471pCO4rIj0jZ+o5Zf4KpWKNRujw8ssC/Gn6Bx2NFsoA0TUrGLQs+Ue32pjGwLyOKNxBw2sNqzlQYzEsKf9tnedgz7hkNTO7Cr/AyKvL+Dc6Mdvn"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "qXMTf2GK86IxXN3OvyPdHPZuIqNGd5pKzEhxB2bQ19iNwVL2FOD+DxEAjHb2BNMRAboKoCf5gv+7V64bnEpfeL0uASADXlXgQPTk41jUO5/d9pqSm8X3ZytAnBcrXWDS"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "qfgc/nzQXObRQSBCwdh7jsIx1vJ5aGwGlGAiS/uWxsJNWydZuPvyahSQjfsMiGGsERAq0zm3CVrNT7yoUXxx1gdX0GOT0VTP3Kv7xLQAcOOj05VeCvuNFxooloIaIzSk"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rFU2XyCwHbE8w4HUgY0/nf6moOusheZVvebgdd0mvWwiuJOe2EvQ1B5n7s9CUgsKGAWypxrQY/Cbn/WFWgrds2ky17dlN+S5bFgVJ62eB0490i6uQNiFAarl/qvUxh56"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rG9sLR/1Jy9eA7wI0Jqqw+9GmVIiAiKSkERpSVF7WOIZiPDAcUiflkC+fsObIkk4Caqvk2gDS8UJsAJhjjhiO1ZrBf6zKgOibc3lbaLX0rd0Q8ilmJxGN2Bi8B/h/T6d"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rXbf+LA1SOLswBSSYU9Id0ZQxSKfFR1DRan+G+DfefA6MCexY4fDcocM/28oORI2CDo37zDHsitVEaO8MFs1CUGg72Qc5046F8apMpKY/xHZGUodoQCf0YPqduIn8Gzd"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rdWOaRKfPuYxk+Hmo1AEi7SQKuHL6N0VYgicwK26nyercD7+B3DecKAQnAaj63HDC3qlfKSNE7/DrC7kloqNQyMk+E4PRCpwLRh3k2nUUJ1L9vA8ITs+7ywJqK/jw7e4"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "reaPH0FgEXT/Xa/HRhrEiOukk6hGid7+l4xrBCO7HBcm3ZuAB1QSfMHrm9SsbGwvEjKmsUXeaUsvazv+EnpmHhv83bAMleJYLZUcU6JSXln7klRSQMkhZB+RuwUdu8Xz"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rk9KnYQvkcPUuRyF+ZNxPaOe6r5VZx984hjFz8pMRgB0bXB+1L6paPAE+a3fa4CpGS2mgnf5rhGJEuuArBinL11hgm6gUnZkcvIOMxoev0DRqLzJbXYUZdBrx+39+Hpt"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rn/K5z3sjLDGjZFjjq78AwrUQV0Ai/j7uv1kRaPAqBIINUuVC40m2U1J6XlA5u+ZGEp0VzznHnk2m/v0MDfU7vCk2j7CM1RuLYPKfaG7SJltFs3G6Qd9r5ErgbP1av4r"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "rqvFZFkoIaO+TUe/T0Qo+aSDEjsy1mEXYiKRK6wXiQZf1zl1wNG2b573BHs8nqCOFbHGXWMAoTvBd6t1wksdxNIuW6oNER5nwU+C9Df/VK+gMUweAsgolI/fJA80X0jr"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "r9mw0Cfs6ZiKSunyWUiym5yJhpywiYslZOm6n/DJqvcNDLvkNe6L866yROadv4J3DRs2UUyUhlFnQHPC1iaDiQDObOHzP/rw2M8dUto+sjkmGJDpY5SyNwg3xTkkuiBw"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "r/odGKpp1nnqlayrPUo3jDNJhSY4ArjHkQM2Qd7PyHO8M5rye4rzznWOrpE3DXC8Fo0X7pMA36Oc6KnoWlWNhxr7iOtVAMtmqFH5aeOle/n6waTGqKCVIMpSxGvVw9RJ"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "sO/EI3tmtpXGEQhPYB2KNF2Ohc5nRyF85daWLifvnnp7M4+zEoFPliALLAO4YmS2FTWYYxzVtk3qtXvpBSy8K3fEqNlRW5KKQXI0kJpTwWUSHhURzXLFea0ZLvwSJ9P1"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "sWnd9YeGmxOjplekYm0Retjv7Q7IFJRD+w19RLd449uV8MeQ4x3JX59dw7w0Qtq9GQ2ENOesSbzM8e/UeTLzerM9JcH333RTznnON3sN7EYIwtsoBQLSDjggTcV8uv0N"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "sbfTjTOb4UNw48OmxrsUjyawrznSDkxIJGObsRxaOIwYhwhs07UYKYxKCPXmYaAwAGv/9HadkFZo3nhZAy3mtvHYrZdL9zpjBnODJvSA75ittV3qJi7AU2loojkLI81I"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "s9Vm0NvsgadI+dpXvVEvN6GFDaAet7tyqT31MOlgf4cp/NPd3j6l4QuyA75HSoJdDX/RGKfmRAOaoutPqN+HhuhRH1Vrom2wbkYLsWZa0RLrx9q5vDf4mgwbiufQM037"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "s+jgP9ksWjqdqBessHcb+pI8ESu/YF92IeXgDIpKmpIpbTFyrw67Ceyzt6x9A1TUEeleiHBgjfTG2TZwPGhuP/9Qwwf1LTAinvfNNLdVXpeTRNCXMGBqEHZpL2nCLjG2"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tFCHFHydzLGQ7xnW7B9fI45q5655ZCtgQaYIKWQWFcoUrAOmqdYHkOh+y7huazCcFow+viVP/vAYQjURC0uN2q971nYSfa6QVlEOCLin1mHHrZa/MA/bSln4utf5WiJi"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tIu68YxClDQjnOj8YAWUMNDnWtrbAITZRydYofIbzD7eEYpfk54lwbl84thFURLSBcxDPBFQwTDR0I0OYkjZqVcbq48imUAEZaJ86wh6SEj01lZ3ZdJ6wSQ0sx2cR792"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tJwD9VGPtwSRxmqHxQETzybOXu9tMewmznTvc7NPWvQ1Gmo7+WNVPUNXIid0AeI1D3cxXLwpsJHZyYFP4tuAArdQPLFaqQcSJoOQnlO7AwJUz1z7cae2JTxIDDBuCukQ"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tKQNn64Ehzeb/uspoI2RCvWX/tYJPF009EcaGm1Md2vX3DlZpjiUYTce8gsaKQDCF+B3PKO8FNrf2KFh3kV1WgETc+JQIEugyALbbqWRKhbebNiEKfhEkwaMMYBMRZkq"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tN8SWqZ8H8r6+joUlsWPtAjKAZe2EliQg5JXkz4wC+pRFPjZ1BCEIsWKPyaZFXyhBZIU8TfPNYX+HaBBZnQXlRvgIBIlZ7qX5PdgqseAFZhmPQaG4WdwSk8FuhpM/yaV"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tPlZ2BNHciXVeu4whSaAmUOyfbwkTjrrMqT6m4nr/F9vJI8VDhWZntuZRAGyUZVUGBD7smMlWjSfCYP8FR9pw9Nr67sUWiMRY41WrVsp2uTzAi7CT9S9QfwXPQ36CGQP"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tbMz44R2z8MnGrTluGovOhfGMgPnKxIiRXpuvTnNiYXF6JSOJabm/SgVE9dNRG/PAsGqWM8/o9q34G/plCmifdtN8/dhCm9KXW5tmHOSfwMKwzBhR2gIXSO0Ml5ZViFD"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tfdYab3ijaQQ+dwjkdfLprk89xgVgGu+lJO2FqG3lp5DLj4W7+MRrEpsyKiywRNhEltFH4gIhgVgPHb4P9vFl1aCrEBeox5xzJ/JYEhv36OmnUB8ZdIDS5c2ZAY/Q2rY"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "tj39DUxdPXBh+UlbtQRp9UiowGC+cQGQOmI6savYoFceAzYtmkgHnb2rWvMfjdMkDRuu5FSWRu5SzIDlOZkIYenI6hysF9i+j0ChK1nPApLBPEGpEwWPRRP+cTqVOJL4"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "t71R+un0+VVVT8Lb0N63XCQTnpm1jv7ZQNRHl5IHAaQc+klqS4ouALa27X9XzD/IFEL8ZTpvzIYekbCvQnAOG2ri7RjFO9+IzVdKw2+3hrDhTb4AVqBHVnXOfftGl2jh"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "t7/v+rDc67JFAofNx4mZ0DC9I2xXFtfXWSZegVMicKkiIh0VqQ+0CZnfjxQM4QODCCmx6YiymoSPDJsZJg6u/C379i1GkJvSoAygt4HlUBsIJdd4nIOoHXFiY+h3B7Ve"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "uDEqCSGuPs8wan97YDZrACYB8zAtjdsNusx0CZgYp629TXgyTJWVMs+dgt9eq3PGBO716zKFQVRX797hTogBW+T1IqTUoPFWwItqNaPCYYkJpFhnf08xxEX8qvZ8FNIp"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "uKoUXRhqeRjsytNUVubCHHX7G2ZM1ZUQdV1+JQeqAJDZFZrU4uCUrtZCCl0eRhLGBPPxM3+hcGuf++Z9wgOdmxVg7kkzO/47WSqNbOM2roRkSsyMs0FEk9kDvQRlu44e"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "uUN+/perw9RXMq7HgkjyWpDYZsvkolLvLHcun4KfyBQq1hnpv4y7zLzF/S4S+pyEEpsIFhMGblx6zu0gAVZFQS4rBRWHBJa/p6N4dID1nERzJmSROQwJxWZO6xvtVMyb"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "uXGtCq03xjvHAYiivBpSwlKywqiXuxTZeY1GHjrvGioXwXizDvNkl21rT+utdVTiFEhL/TnCSYRgLG2sBCElg/WDfmPg103HjMo0qs1Uu0NRINnXammEOK6anN7+4hbQ"
      },
      "weight": "1"
    },
    {
      "key": {
        "bn254": "uZwcJrjCZ4O7mHdyABa+nSGv62Hblc3LcSTUROuXBUzhR8N6DILssBAmcqzXx617E9vC51OIzGnDEoPTG2zCr3BvWMXFxBZSLEpPDjuG3ZoEjFfdpOpq/z0GWZwIj40y"
      },
      "weight": "1"
    }
  ],
  "leaderSelection": {
    "roundRobin": {}
  },
  "chainId": "1337",
  "forkNumber": "33",
  "firstBlock": "79",
  "protocolVersion": 1,
  "attesters": [
    {
      "key": {
        "secp256k1": "AwMPIv4PxWUCuU5n3qsJRQ+74rtNKVLRqsseg1PCYSwT"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AwPS+QQyfUVTuWk9vdker4PiS5PYhS63xDTjhGp20sec"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AwRmJKhoxVyT51hZArKTYBtmphcO/WfAqxtJl/oWHiK5"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AgjZ46s5rUzWqBR9LL0z/rWpf7q6pBItFYSKhrN9z/Kj"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Awk7ynBsC8LrHC+HnYHOWaSm4orL1v9HcA31Z73wdnmq"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Awt0v0xezRKCdb3qQ2e3Bp87I3xvu5ktMoXwV5gykWkE"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Awy2P2YeE788EXCrBiOHRDQs3K+FM8rniTAT4z7HbGza"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AhJ/iO6wYnP4WF5pKdWLnqjYsUUgRwviaMMMkRsuIU7X"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AhWvTWv4Lfla+eQQjtfinPu2Ytr30kbzw684VpS4pAQR"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AhgvOzlVgJHclE6W6J709xyCnNZn8Ga4W8fTuU/Yi8qy"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Axqm1RXQflXWPK+C3spvPMaJke3EPsviuBjP8z7MJh5v"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Axvkr7lY1+4FxIBpIt1/rqwDAzR/8VvenJmV6o5/R3xu"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AhzJZWumo8xqCXnQw4WpW41K9vj1gsxeFMZKve/CcoJH"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AyLFZxz9gdlgPyXghTwLdqB3j8fFKAwzt8knDQc0bPbY"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AiMAHaYZsh9kQMnYhksyyn6imJ6+5BsHQGUlMC7vs/+e"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AyefANs0p/Qbun2vSyeMBSiKyr5Znkb/V6bk7TuSJQfv"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ai5JzzrTdSS3943eUihtNdmLrJ71V0zb0UqiSG/aUeuN"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ay5wGJx/s3BFxfW5rVmI7QfcU7Z3Xcr6ulvrFeIZTnpI"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ajcx96MYdF4fJSQRa39kc4UcfPEY6LMNzuvmtEfFYd9C"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AzpyZmWqwl7rKERXbzNIVfxbeEaA17z881aYHxa1pQR9"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AzrLNqBua5qj8DjT2MYeKKSC0O0H/ABkQl75IEWbBBKH"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ajt6fMj12X05VJGpI+5Fnlz3jKmA5ELSC0aQF0swnfiA"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Aj3pPOM3xweg9XntYKXbWGsfI8JsGuvhvCdUx0uAnUD4"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Az+spIQ4D2yAnu1DZatXtDFXYUEMICHzQ6udydDWT4tV"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A0LPc8zBQLXIyiBbSODs3bhOHvdbpToAOIxM/AQwfGqK"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AkMnVogco7FRH5FP4+Fwh/c2eEoZAj7WN2sIoVMqtZqM"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A0OC6ttpt4K06zuKWpc2zlGIoACGD5m/BJeTntV0tS+T"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AkhyvJxeUvA5x2hETy+ZYNGOj83y7x3PchhCdpccHFhp"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Akmv4oM5SOd1hd7rzds3VkviCl3sp8EyAASHMp4eCTsf"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A0tX4MmamZEbN5DN23A6mz1qUoEx4Lylw6yyfj1nK2b0"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A00/1T83IZeUAoAeWAU9HGe3piIpv/t+M8m6FMooUWVp"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ak1/dOKqoCTjzuJGpz+43KqAv7cHOSfS2z9PqXdeZ0JF"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ak7m6p2x1Iph3mIHQDQXzh7fnyR5+wrHtQygOuD3g1Dh"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AlAFJ0NT0kI+C6p79DeCHwDoTSgviKwTJO/B4ULbCAkV"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A1X6/NV64A7MQYSgMiL3LFrNMxadKnshf/w5DFVsSj9Z"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Almm6VGpq2dEARt5jbcKMoD3I3ksycsnNbqlH2sCOmLx"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A1zxOnaCLPjhMWUKFuZmwJhgn0jjLe9GgmedDvg+zaU7"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AmJe8WhZBVNNVflvHMAE2VpiThDSn2HOvRjKPngNmrtR"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AmPyKNFDO+RYR6EOBeNI1+s9yH1oBr+jiDfrELKaWbuB"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A2aY+7wqpr+gcqUFNjGScSJ9T0JuxPolDNbDUEf4jEFW"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AmguSF4Gyijk3tIhrMFftO1J3Y9Hc8AJlAwg9etNYxnt"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A2xN10Z9+pUvFQJTVfGnwUwZOXEJIKYKDPwQ1vB16z5a"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A22WA5zIfvvof8ECE8pePSy680YpDooLZI3WIIBoTVS4"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AnDZxUA4HkG9g20rNDafCNMKWmIrSAyzCuUwkAn8407N"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A3DhzyBg2aCIsulFQXdg2fyou5U+OhvOaiZdHV7122I4"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AnGaopCRrRPWdKlqChjp2vDVJbpFidKrGnNnNfjDiOBa"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A3S5YbcC7J6Pyc58taQgIX7qbQkE5/XgvGQqVSK1sN24"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AnXcraGCUnImiQMNhcQCk2qrJBwwfnamXy7ghGMZ+j8Q"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A3giMU9yu06pne3pJLSWz90551uHwi1RMMLjohiKT9XP"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A3mL4UoTHfthnkxd35VPYkd7TWNPERJsLH3DFW+Gnvcs"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A3/0lAPKD3mrROC+BUhwAwFMai5uX5WMnGOPBlXpMANP"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A4FGk91HTBJvhWE6z+QGN8r9eZa8wj2tOuEFQbiItleg"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AopptUwCuXVDRDa2O4lSEx3bwHJoC+elsRx6MFAatGuL"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A42WozQTnJQOf/tybEdw6C1GzpYrw+Yzs/wf4c5KADQE"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ao4sMoJAMPA4SVsY88vvBMOYX4vJtAJWE3g2iCx0aSPu"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Ao5DeKHkC0VneUDzIGar+jv30V4ettLr6c6qu2eE3mfq"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A47GLAK2zX+e+A/Zgi5ojk6eFXeIhQiN7yauSCzU6qPS"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A5BIy9ykekOnRvFiiz/J/+FD0648JLt5KT84SKzDUooE"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A5MXjUyqyk+fmnxp0vlsfH/v+HIGT5aBkMelUyMLyXjD"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "ApTsP9bmwZaN++PIxZurDhS6XiA/Rl7gGNg/0nJonqFI"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AplZ2wLKv6S5luAh2+BL0UQVnpGr0oy6Yh+kvGvKD6gC"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A54GtSZSqpF7KT8BEIEk9y8EtlL0q7DeeVM2aKKe32a3"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AqCPGqTrX4Rk6a0tChB+RirjL/St6bxwtz5rOjyZy7eh"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A6OK1x9g3RnqFVkwCMFEchlVPtl+Ztzk7Slxg4lKjNnz"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A6VV/rOR6IJcyieCyn/tWkQFcXHifZkZT0UkF7dQghWJ"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A6X8hMj1Wq2o+qxtqEPLnvM1A1ijhKRSDpEAMLvcg9Bd"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AqX/pFD5zsgrz3hAcJIMxTFAcCEiddsZVOiTicMyyUi/"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Aqh56+Llu3ZMyOr8tESCtYlXvAvEgOy1G/9oFITIKzcO"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Aqmh3B8hc8dwSe0TluYBQiDjhK229v8uSmiMdGbar8eK"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A6o2L9fOZFmzQSjQd3qQYAvQMk9+aou4YzLwljuYcxAv"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AquzggKOOz1CKVWNetmhcZJfWFmDySONvEBabh+mli50"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "Aq6dQdItDxex2wSNqzgyyHFeyE8LsnXKzKPumzbUwKss"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "ArtU3Z56n/gTr4API5I6+/qo2uxwBHrsqN9NBtcYAVhg"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A70nDNL2lROSmP5KkkB+gn/FMKAAluWSk/y7rMI8WJS7"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A8E8JhezIpt0cZQqpl7Xa5VAI1hFFmlAHJpCVULowXMx"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A8HFdr9XrFCsFXwv4ANcNye3yi9noEgjjYljhYXEm4AY"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A8Po2k6EurVDz9s8VEQrx/bBxFJXCMNSraqRYP+8zpwL"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AsRSJCuJjQ2f1k4CxoqCcFpU7FTAFGN2aTo8XzMfBNeF"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AsSNJA2xrI0iyseA4bMLBf7s1fig2hx38H/ehOuFDW28"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A87X1bzb/8/b59r9IjgvE667rhZBk6AD7vDmEQW6rwYL"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A9IUM4pDOeWuu1Yc5KJbNAyKseOZELGiGQIQie/BYoqE"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AtNhG9qRRojhjWHCPF4bWlsIB0ggHx7Mymg7fHVAvx0V"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A9YcNRT4U1I2QMtt7yk57ir5g531rGgSKAggdvHnWNA+"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A9eWvOp7JKLkYda0GBp3JqaNvEsjgGJtFruwpT3t8BE+"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AtltlBBL/CiLKdyVL//xwLoDNnHZlEr+UNcaCn7lfNB5"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AtpR1BQ0GRjD4ojrQZZ1mNmHSDMBSzBQ5XEt7Uv64Soh"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A90dgtMI7fzrIY5TJUQe0f9N5ng2T16Z+eEFZA6+tAZ9"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A95PKf2uigP3Gsvrjv1MDtjFiPirko4PxOfSUkGdr6Ni"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A+fXVqn93ptpMWdOmDbbjBhrUdQ+2TTy6dHU389kHVW0"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AuuGKzcBRVhtclMxswCzQ4CEIS+i3tsozjzl7zAVCNEK"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/C/f0BmJHwR2iAF9umkQYmTaLlCdwIGHYX+l0yEmJez"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AvLLby5aj7mRJU5DNYunE9Rvjvrp5TEvGZSdvZBQcWTU"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AvNT80w6/R5PqYqPUZt3V07D7j+6Evr5PrYKDPR1ZycF"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/aoZmjplVTOxT7cBMRIzd/qnJ15I4Kq61ZsxDk4wXm0"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/lakrW9Np9S0lNhZekNg1TiZ5hanFprAtKIAHaDqA83"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/r4VJOKCmRAdb11K8vGYWC0QWrvX0kGMQYd55iLOVlp"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/tEM49Azz2aT089Z7P7Lgzc7EzVZs101VW5msyhmA7J"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "AvuL7S2VR4rwQQxiEf3RBvl2Af5Kt+Ex6Zy/DjlMnrtI"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/uNb+JDbANGe8lgmvnyO7xDNnOxxiY/vbaa5TQ/P+ry"
      },
      "weight": "1"
    },
    {
      "key": {
        "secp256k1": "A/59U82NeQcQvzh1d0b+vIh+OuzUw/viVmBplcIPfjV1"
      },
      "weight": "1"
    }
  ]
}
"#;

#[test]
fn timeout_qc_example() {
    zksync_concurrency::testonly::abort_on_panic();
    let d = Deserialize { deny_unknown_fields: true };
    let mut before : validator::TimeoutQC = d.proto_fmt_from_json(BEFORE).unwrap();
    let after : validator::TimeoutQC = d.proto_fmt_from_json(AFTER).unwrap();
    let commit : validator::Signed<validator::ReplicaTimeout> = d.proto_fmt_from_json(COMMIT).unwrap();
    let genesis : validator::Genesis = d.proto_fmt_from_json(GENESIS).unwrap();
    before.verify_signature(&genesis).unwrap();
    before.add(&commit,&genesis).unwrap();
    before.verify_signature(&genesis).unwrap();
}

#[tokio::test]
async fn timeout_yield_new_view_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let cur_view = util.replica.view_number;
        let replica_timeout = util.new_replica_timeout(ctx).await;
        assert_eq!(util.replica.phase, validator::Phase::Timeout);

        let new_view = util
            .process_replica_timeout_all(ctx, replica_timeout.clone())
            .await
            .msg;
        assert_eq!(util.replica.view_number, cur_view.next());
        assert_eq!(util.replica.phase, validator::Phase::Prepare);
        assert_eq!(new_view.view().number, cur_view.next());

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_non_validator_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_timeout(ctx, non_validator_key.sign_msg(replica_timeout))
            .await;

        assert_matches!(
            res,
            Err(timeout::Error::NonValidatorSigner { signer }) => {
                assert_eq!(*signer, non_validator_key.public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_timeout_old() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_timeout = util.new_replica_timeout(ctx).await;
        replica_timeout.view.number = validator::ViewNumber(util.replica.view_number.0 - 1);
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout))
            .await;

        assert_matches!(
            res,
            Err(timeout::Error::Old { current_view }) => {
                assert_eq!(current_view, util.replica.view_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_duplicate_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;

        let replica_timeout = util.new_replica_timeout(ctx).await;
        assert!(util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await
            .unwrap()
            .is_none());

        // Processing twice same ReplicaTimeout for same view gets DuplicateSigner error
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::DuplicateSigner {
                message_view,
                signer
            })=> {
                assert_eq!(message_view, util.replica.view_number);
                assert_eq!(*signer, util.owner_key().public());
            }
        );

        // Processing twice different ReplicaTimeout for same view gets DuplicateSigner error too
        // replica_timeout.high_vote = None;
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::DuplicateSigner {
                message_view,
                signer
            })=> {
                assert_eq!(message_view, util.replica.view_number);
                assert_eq!(*signer, util.owner_key().public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let msg = util.new_replica_timeout(ctx).await;
        let mut replica_timeout = util.owner_key().sign_msg(msg);
        replica_timeout.sig = ctx.rng().gen();

        let res = util.process_replica_timeout(ctx, replica_timeout).await;
        assert_matches!(res, Err(timeout::Error::InvalidSignature(..)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_invalid_message() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.view.genesis = ctx.rng().gen();
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::BadView(_)
            ))
        );

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.high_vote = Some(ctx.rng().gen());
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::InvalidHighVote(_)
            ))
        );

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.high_qc = Some(ctx.rng().gen());
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::InvalidHighQC(_)
            ))
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_num_received_below_threshold() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        for i in 0..util.genesis().validators.quorum_threshold() as usize - 1 {
            assert!(util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await
                .unwrap()
                .is_none());
        }
        let res = util
            .process_replica_timeout(
                ctx,
                util.keys[util.genesis().validators.quorum_threshold() as usize - 1]
                    .sign_msg(replica_timeout.clone()),
            )
            .await
            .unwrap()
            .unwrap()
            .msg;
        assert_matches!(res.justification, validator::ProposalJustification::Timeout(qc) => {
            assert_eq!(qc.view, replica_timeout.view);
        });
        for i in util.genesis().validators.quorum_threshold() as usize..util.keys.len() {
            let res = util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await;
            assert_matches!(res, Err(timeout::Error::Old { .. }));
        }

        Ok(())
    })
    .await
    .unwrap();
}

/// Check all ReplicaTimeout are included for weight calculation
/// even on different messages for the same view.
#[tokio::test]
async fn timeout_weight_different_messages() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let view = util.view();
        util.produce_block(ctx).await;

        let replica_timeout = util.new_replica_timeout(ctx).await;
        util.replica.phase = validator::Phase::Prepare; // To allow processing of proposal later.
        let proposal = replica_timeout.clone().high_vote.unwrap().proposal;

        // Create a different proposal for the same view
        let mut different_proposal = proposal;
        different_proposal.number = different_proposal.number.next();

        // Create a new ReplicaTimeout with the different proposal
        let mut other_replica_timeout = replica_timeout.clone();
        let mut high_vote = other_replica_timeout.high_vote.clone().unwrap();
        high_vote.proposal = different_proposal;
        let high_qc = util
            .new_commit_qc(ctx, |msg: &mut validator::ReplicaCommit| {
                msg.proposal = different_proposal;
                msg.view = view;
            })
            .await;
        other_replica_timeout.high_vote = Some(high_vote);
        other_replica_timeout.high_qc = Some(high_qc);

        let validators = util.keys.len();

        // half of the validators sign replica_timeout
        for i in 0..validators / 2 {
            util.process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await
                .unwrap();
        }

        let mut res = None;
        // The rest of the validators until threshold sign other_replica_timeout
        for i in validators / 2..util.genesis().validators.quorum_threshold() as usize {
            res = util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(other_replica_timeout.clone()))
                .await
                .unwrap();
        }

        assert_matches!(res.unwrap().msg.justification, validator::ProposalJustification::Timeout(qc) => {
            assert_eq!(qc.view, replica_timeout.view);
            assert_eq!(qc.high_vote(util.genesis()).unwrap(), proposal);
        });

        Ok(())
    })
    .await
    .unwrap();
}

/// Check that leader won't accumulate undefined amount of messages if
/// it's spammed with ReplicaTimeout messages for future views
#[tokio::test]
async fn replica_timeout_limit_messages_in_memory() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_timeout = util.new_replica_timeout(ctx).await;
        let mut view = util.view();
        // Spam it with 200 messages for different views
        for _ in 0..200 {
            replica_timeout.view = view;
            let res = util
                .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
                .await;
            assert_matches!(res, Ok(_));
            view.number = view.number.next();
        }

        // Ensure only 1 timeout_qc is in memory, as the previous 199 were discarded each time
        // a new message was processed
        assert_eq!(util.replica.timeout_qcs_cache.len(), 1);

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_timeout_filter_functions_test() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        let msg = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout.clone(),
            ));

        // Send a msg with invalid signature
        let mut invalid_msg = msg.clone();
        invalid_msg.sig = ctx.rng().gen();
        util.send(invalid_msg);

        // Send a correct message
        util.send(msg.clone());

        // Validate only correct message is received
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg
        );

        // Send a msg with view number = 2
        let mut replica_timeout_from_view_2 = replica_timeout.clone();
        replica_timeout_from_view_2.view.number = validator::ViewNumber(2);
        let msg_from_view_2 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_2,
            ));
        util.send(msg_from_view_2);

        // Send a msg with view number = 4, will prune message from view 2
        let mut replica_timeout_from_view_4 = replica_timeout.clone();
        replica_timeout_from_view_4.view.number = validator::ViewNumber(4);
        let msg_from_view_4 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_4,
            ));
        util.send(msg_from_view_4.clone());

        // Send a msg with view number = 3, will be discarded, as it is older than message from view 4
        let mut replica_timeout_from_view_3 = replica_timeout.clone();
        replica_timeout_from_view_3.view.number = validator::ViewNumber(3);
        let msg_from_view_3 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_3,
            ));
        util.send(msg_from_view_3);

        // Validate only message from view 4 is received
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_view_4
        );

        // Send a msg from validator 0
        let msg_from_validator_0 = util.keys[0].sign_msg(validator::ConsensusMsg::ReplicaTimeout(
            replica_timeout.clone(),
        ));
        util.send(msg_from_validator_0.clone());

        // Send a msg from validator 1
        let msg_from_validator_1 = util.keys[1].sign_msg(validator::ConsensusMsg::ReplicaTimeout(
            replica_timeout.clone(),
        ));
        util.send(msg_from_validator_1.clone());

        // Validate both are present in the inbound_channel
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_validator_0
        );
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_validator_1
        );

        Ok(())
    })
    .await
    .unwrap();
}
