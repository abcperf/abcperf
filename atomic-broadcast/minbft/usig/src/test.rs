#[macro_export]
macro_rules! tests {
    ($new_usig:expr) => {
        use usig::{
            Counter as _, ReplicaId, SignHalf as _, Usig as _, UsigError,
            VerifyHalf as _,
        };

        const MESSAGE_EMPTY: &'static [u8] = b"";
        const MESSAGE_1: &'static [u8] = b"message one";
        const MESSAGE_2: &'static [u8] = b"message two";
        const ID: ReplicaId = ReplicaId::from_u64(0);

        #[test]
        fn as_ref() {
            struct Input<F: Fn()>(F);
            impl<F: Fn()> AsRef<[u8]> for Input<F> {
                fn as_ref(&self) -> &[u8] {
                    self.0();
                    &[]
                }
            }

            let into_called = ::std::sync::atomic::AtomicUsize::new(0);
            let called = || {
                into_called.fetch_add(1, ::std::sync::atomic::Ordering::SeqCst);
            };

            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));

            let signature = usig.sign(Input(called)).unwrap();
            assert_eq!(into_called.load(::std::sync::atomic::Ordering::SeqCst), 1);

            assert!(usig.verify(ID, Input(called), &signature).is_ok());
            assert_eq!(into_called.load(::std::sync::atomic::Ordering::SeqCst), 2);
        }

        #[test]
        fn valid() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            let signature = usig.sign(MESSAGE_1).unwrap();
            assert!(usig.verify(ID, MESSAGE_1, &signature).is_ok());
        }

        #[test]
        fn empty_msg() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            let signature = usig.sign(MESSAGE_EMPTY).unwrap();
            assert!(usig.verify(ID, MESSAGE_EMPTY, &signature).is_ok());
        }

        #[test]
        fn double_sig() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            let signature_1 = usig.sign(MESSAGE_1).unwrap();
            let signature_2 = usig.sign(MESSAGE_1).unwrap();
            assert!(usig.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(usig.verify(ID, MESSAGE_1, &signature_2).is_ok());
            assert_eq!(signature_1.counter() + 1, signature_2.counter());
        }

        #[test]
        fn valid_iteration() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            let initial_count = usig.sign(MESSAGE_1).unwrap().counter();
            let mut prev_counter = initial_count;
            for _ in 0..100 {
                let counter = usig.sign(MESSAGE_1).unwrap().counter();
                assert_eq!(prev_counter + 1, counter);
                prev_counter = counter;
            }
            let signature = usig.sign(MESSAGE_2).unwrap();
            assert_eq!(initial_count + 101, signature.counter());
            assert!(usig.verify(ID, MESSAGE_2, &signature).is_ok());
        }

        #[test]
        fn attest_after() {
            let mut usig = $new_usig;
            let signature = usig.sign(MESSAGE_1).unwrap();
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            assert!(usig.verify(ID, MESSAGE_1, &signature).is_ok());
        }

        #[test]
        fn id_overwrite() {
            let mut usig_1 = $new_usig;
            let mut usig_2 = $new_usig;
            let mut usig_3 = $new_usig;
            let signature_1 = usig_1.sign(MESSAGE_1).unwrap();
            let signature_2 = usig_2.sign(MESSAGE_2).unwrap();

            assert!(matches!(
                usig_3.verify(ID, MESSAGE_1, &signature_1),
                Err(UsigError::UnknownId(ID))
            ));
            assert!(matches!(
                usig_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::UnknownId(ID))
            ));

            assert!(usig_3.add_remote_party(ID, usig_1.attest().unwrap()));

            assert!(usig_3.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(matches!(
                usig_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));

            assert!(usig_3.add_remote_party(ID, usig_2.attest().unwrap()));

            assert!(matches!(
                usig_3.verify(ID, MESSAGE_1, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(usig_3.verify(ID, MESSAGE_2, &signature_2).is_ok());

            assert!(usig_3.add_remote_party(ID, usig_1.attest().unwrap()));

            assert!(usig_3.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(matches!(
                usig_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn mixed() {
            let mut usig_1 = $new_usig;
            let mut usig_2 = $new_usig;
            let mut usig_3 = $new_usig;
            assert!(usig_3.add_remote_party(ReplicaId::from_u64(1), usig_1.attest().unwrap()));
            assert!(usig_3.add_remote_party(ReplicaId::from_u64(2), usig_2.attest().unwrap()));
            let signature_1 = usig_1.sign(MESSAGE_1).unwrap();
            let signature_2 = usig_2.sign(MESSAGE_2).unwrap();
            assert!(usig_3
                .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature_1)
                .is_ok());
            assert!(usig_3
                .verify(ReplicaId::from_u64(2), MESSAGE_2, &signature_2)
                .is_ok());
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(2), MESSAGE_1, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(1), MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(1), MESSAGE_2, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(2), MESSAGE_1, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                usig_3.verify(ReplicaId::from_u64(2), MESSAGE_2, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn no_id() {
            let mut usig = $new_usig;
            let signature = usig.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                usig.verify(ID, MESSAGE_1, &signature),
                Err(UsigError::UnknownId(ID))
            ));
        }

        #[test]
        fn no_id_1() {
            let mut usig = $new_usig;
            let signature = usig.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                usig.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature),
                Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(1)
            ));
        }

        #[test]
        fn wrong_id() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ReplicaId::from_u64(1), attestation));
            let signature = usig.sign(MESSAGE_1).unwrap();
            assert!(usig
                .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature)
                .is_ok());
            assert!(matches!(
                usig.verify(ReplicaId::from_u64(0), MESSAGE_1, &signature),
                Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(0)
            ));
        }

        #[test]
        fn wrong_key() {
            let mut usig_1 = $new_usig;
            let mut usig_2 = $new_usig;
            let attestation = usig_2.attest().unwrap();
            assert!(usig_2.add_remote_party(ID, attestation));
            let signature = usig_1.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                usig_2.verify(ID, MESSAGE_1, &signature),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn wrong_message() {
            let mut usig = $new_usig;
            let attestation = usig.attest().unwrap();
            assert!(usig.add_remote_party(ID, attestation));
            let signature = usig.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                usig.verify(ID, MESSAGE_2, &signature),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn as_ref_split() {
            struct Input<F: Fn()>(F);
            impl<F: Fn()> AsRef<[u8]> for Input<F> {
                fn as_ref(&self) -> &[u8] {
                    self.0();
                    &[]
                }
            }

            let into_called = ::std::sync::atomic::AtomicUsize::new(0);
            let called = || {
                into_called.fetch_add(1, ::std::sync::atomic::Ordering::SeqCst);
            };

            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));

            let signature = sign.sign(Input(called)).unwrap();
            assert_eq!(into_called.load(::std::sync::atomic::Ordering::SeqCst), 1);

            assert!(verify.verify(ID, Input(called), &signature).is_ok());
            assert_eq!(into_called.load(::std::sync::atomic::Ordering::SeqCst), 2);
        }

        #[test]
        fn valid_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            let signature = sign.sign(MESSAGE_1).unwrap();
            assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
        }

        #[test]
        fn empty_msg_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            let signature = sign.sign(MESSAGE_EMPTY).unwrap();
            assert!(verify.verify(ID, MESSAGE_EMPTY, &signature).is_ok());
        }

        #[test]
        fn double_sig_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            let signature_1 = sign.sign(MESSAGE_1).unwrap();
            let signature_2 = sign.sign(MESSAGE_1).unwrap();
            assert!(verify.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(verify.verify(ID, MESSAGE_1, &signature_2).is_ok());
            assert_eq!(signature_1.counter() + 1, signature_2.counter());
        }

        #[test]
        fn valid_iteration_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            let initial_count = sign.sign(MESSAGE_1).unwrap().counter();
            let mut prev_counter = initial_count;
            for _ in 0..100 {
                let counter = sign.sign(MESSAGE_1).unwrap().counter();
                assert_eq!(prev_counter + 1, counter);
                prev_counter = counter;
            }
            let signature = sign.sign(MESSAGE_2).unwrap();
            assert_eq!(initial_count + 101, signature.counter());
            assert!(verify.verify(ID, MESSAGE_2, &signature).is_ok());
        }

        #[test]
        fn attest_after_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let signature = sign.sign(MESSAGE_1).unwrap();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            assert!(verify.verify(ID, MESSAGE_1, &signature).is_ok());
        }

        #[test]
        fn id_overwrite_split() {
            let (mut sign_1, _verify_1) = $new_usig.split();
            let (mut sign_2, _verify_2) = $new_usig.split();
            let (_sign_3, mut verify_3) = $new_usig.split();
            let signature_1 = sign_1.sign(MESSAGE_1).unwrap();
            let signature_2 = sign_2.sign(MESSAGE_2).unwrap();

            assert!(matches!(
                verify_3.verify(ID, MESSAGE_1, &signature_1),
                Err(UsigError::UnknownId(ID))
            ));
            assert!(matches!(
                verify_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::UnknownId(ID))
            ));

            assert!(verify_3.add_remote_party(ID, sign_1.attest().unwrap()));

            assert!(verify_3.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(matches!(
                verify_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));

            assert!(verify_3.add_remote_party(ID, sign_2.attest().unwrap()));

            assert!(matches!(
                verify_3.verify(ID, MESSAGE_1, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(verify_3.verify(ID, MESSAGE_2, &signature_2).is_ok());

            assert!(verify_3.add_remote_party(ID, sign_1.attest().unwrap()));

            assert!(verify_3.verify(ID, MESSAGE_1, &signature_1).is_ok());
            assert!(matches!(
                verify_3.verify(ID, MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn mixed_split() {
            let (mut sign_1, _verify_1) = $new_usig.split();
            let (mut sign_2, _verify_2) = $new_usig.split();
            let (_sign_3, mut verify_3) = $new_usig.split();
            assert!(verify_3.add_remote_party(ReplicaId::from_u64(1), sign_1.attest().unwrap()));
            assert!(verify_3.add_remote_party(ReplicaId::from_u64(2), sign_2.attest().unwrap()));
            let signature_1 = sign_1.sign(MESSAGE_1).unwrap();
            let signature_2 = sign_2.sign(MESSAGE_2).unwrap();
            assert!(verify_3
                .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature_1)
                .is_ok());
            assert!(verify_3
                .verify(ReplicaId::from_u64(2), MESSAGE_2, &signature_2)
                .is_ok());
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(2), MESSAGE_1, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(1), MESSAGE_2, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(1), MESSAGE_2, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(2), MESSAGE_1, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature_2),
                Err(UsigError::InvalidSignature)
            ));
            assert!(matches!(
                verify_3.verify(ReplicaId::from_u64(2), MESSAGE_2, &signature_1),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn no_id_split() {
            let (mut sign, verify) = $new_usig.split();
            let signature = sign.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                verify.verify(ID, MESSAGE_1, &signature),
                Err(UsigError::UnknownId(ID))
            ));
        }

        #[test]
        fn no_id_1_split() {
            let (mut sign, verify) = $new_usig.split();
            let signature = sign.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                verify.verify(ReplicaId::from_u64(1), MESSAGE_1, &signature),
                Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(1)
            ));
        }

        #[test]
        fn wrong_id_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ReplicaId::from_u64(1), attestation));
            let signature = sign.sign(MESSAGE_1).unwrap();
            assert!(verify
                .verify(ReplicaId::from_u64(1), MESSAGE_1, &signature)
                .is_ok());
            assert!(matches!(
                verify.verify(ReplicaId::from_u64(0), MESSAGE_1, &signature),
                Err(UsigError::UnknownId(id)) if id == ReplicaId::from_u64(0)
            ));
        }

        #[test]
        fn wrong_key_split() {
            let (mut sign_1, _verify_1) = $new_usig.split();
            let (mut sign_2, mut verify_2) = $new_usig.split();
            let attestation = sign_2.attest().unwrap();
            assert!(verify_2.add_remote_party(ID, attestation));
            let signature = sign_1.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                verify_2.verify(ID, MESSAGE_1, &signature),
                Err(UsigError::InvalidSignature)
            ));
        }

        #[test]
        fn wrong_message_split() {
            let (mut sign, mut verify) = $new_usig.split();
            let attestation = sign.attest().unwrap();
            assert!(verify.add_remote_party(ID, attestation));
            let signature = sign.sign(MESSAGE_1).unwrap();
            assert!(matches!(
                verify.verify(ID, MESSAGE_2, &signature),
                Err(UsigError::InvalidSignature)
            ));
        }
    };
}
