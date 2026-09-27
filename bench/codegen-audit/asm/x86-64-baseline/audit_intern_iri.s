audit_intern_iri:
.Lfunc_begin21:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	pushq	%r15
	.cfi_def_cfa_offset 24
	pushq	%r14
	.cfi_def_cfa_offset 32
	pushq	%r13
	.cfi_def_cfa_offset 40
	pushq	%r12
	.cfi_def_cfa_offset 48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	subq	$216, %rsp
	.cfi_def_cfa_offset 272
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%rdi, 8(%rsp)
	movabsq	$2611923443488327891, %r12
	movabsq	$1376283091369227076, %r15
	leaq	(%rsi,%rdx), %rbx
	movq	%rbx, %rcx
	movq	%rdx, 16(%rsp)
	.p2align	4
.LBB21_1:
	cmpq	%rcx, %rsi
	je	.LBB21_151
	movsbl	-1(%rcx), %eax
	testl	%eax, %eax
	js	.LBB21_4
	decq	%rcx
	cmpl	$47, %eax
	jne	.LBB21_12
	jmp	.LBB21_13
	.p2align	4
.LBB21_4:
	movzbl	-2(%rcx), %r8d
	cmpb	$-64, %r8b
	jge	.LBB21_5
	movzbl	-3(%rcx), %r9d
	cmpb	$-64, %r9b
	jge	.LBB21_7
	movzbl	-4(%rcx), %edi
	addq	$-4, %rcx
	andl	$7, %edi
	shll	$6, %edi
	andl	$63, %r9d
	orl	%edi, %r9d
	jmp	.LBB21_9
.LBB21_5:
	addq	$-2, %rcx
	andl	$31, %r8d
	jmp	.LBB21_10
.LBB21_7:
	addq	$-3, %rcx
	andl	$15, %r9d
.LBB21_9:
	shll	$6, %r9d
	andl	$63, %r8d
	orl	%r9d, %r8d
.LBB21_10:
	shll	$6, %r8d
	andl	$63, %eax
	orl	%r8d, %eax
	cmpl	$47, %eax
	je	.LBB21_13
.LBB21_12:
	cmpl	$35, %eax
	jne	.LBB21_1
.LBB21_13:
	subq	%rsi, %rcx
	incq	%rcx
	jne	.LBB21_14
.LBB21_151:
	xorl	%r9d, %r9d
	movq	%rdx, %r14
	movq	%rsi, %rbp
	movq	%rsi, %r10
	movq	%r15, %rsi
	movq	%r12, %r13
	movabsq	$-6626703657320631856, %rcx
.LBB21_32:
	cmpq	$17, %r14
	movq	%r14, 40(%rsp)
	jae	.LBB21_33
	cmpq	$7, %r14
	jbe	.LBB21_41
	movq	(%r10), %rdi
	xorq	%r12, %rdi
	movq	16(%rsp), %rcx
	movq	-8(%rbp,%rcx), %r8
	jmp	.LBB21_46
.LBB21_33:
	leaq	-17(%r14), %r11
	cmpq	$16, %r11
	jae	.LBB21_35
	movq	%r12, %rdi
	movq	%r15, %r8
	jmp	.LBB21_37
.LBB21_14:
	cmpq	%rdx, %rcx
	jae	.LBB21_15
	cmpb	$-65, (%rsi,%rcx)
	jle	.LBB21_16
	movq	%rcx, %r9
	jmp	.LBB21_19
.LBB21_41:
	cmpq	$3, %r14
	jbe	.LBB21_42
	movl	(%r10), %edi
	movq	16(%rsp), %rcx
	movl	-4(%rbp,%rcx), %r8d
	xorq	%r12, %rdi
	jmp	.LBB21_46
.LBB21_35:
	movq	%r11, %r14
	shrq	$4, %r14
	incq	%r14
	andq	$-2, %r14
	movq	%r12, %rdi
	movq	%r15, %r8
	.p2align	4
.LBB21_36:
	xorq	(%r10), %rdi
	movq	8(%r10), %rax
	xorq	%rcx, %rax
	mulq	%rdi
	movq	%rdx, %rdi
	xorq	%rax, %rdi
	xorq	16(%r10), %r8
	movq	24(%r10), %rax
	addq	$32, %r10
	xorq	%rcx, %rax
	mulq	%r8
	movq	%rdx, %r8
	xorq	%rax, %r8
	addq	$-2, %r14
	jne	.LBB21_36
.LBB21_37:
	testb	$16, %r11b
	movq	16(%rsp), %rcx
	jne	.LBB21_39
	xorq	(%r10), %rdi
	movq	8(%r10), %rax
	movabsq	$-6626703657320631856, %rdx
	xorq	%rdx, %rax
	mulq	%rdi
	movq	%r8, %rdi
	movq	%rdx, %r8
	xorq	%rax, %r8
.LBB21_39:
	xorq	-16(%rbp,%rcx), %rdi
	xorq	-8(%rbp,%rcx), %r8
	jmp	.LBB21_47
.LBB21_15:
	movq	%rdx, %r9
	jne	.LBB21_16
.LBB21_19:
	leaq	(%rsi,%r9), %r10
	movq	%rdx, %r14
	subq	%r9, %r14
	cmpq	$17, %r9
	jae	.LBB21_20
	cmpq	$7, %r9
	movabsq	$-6626703657320631856, %rcx
	jbe	.LBB21_27
	movq	%rsi, %rbp
	movq	(%rsi), %r13
	xorq	%r12, %r13
	movq	-8(%r10), %rsi
	jmp	.LBB21_31
.LBB21_20:
	leaq	-17(%r9), %rdi
	movq	%rsi, %r11
	movq	%r12, %r13
	movq	%r15, %rsi
	movq	%r11, %rbp
	cmpq	$16, %rdi
	movabsq	$-6626703657320631856, %rcx
	jb	.LBB21_23
	movq	%rdi, %r8
	shrq	$4, %r8
	incq	%r8
	andq	$-2, %r8
	movq	%r12, %r13
	movq	%r15, %rsi
	movq	%rbp, %r11
	.p2align	4
.LBB21_22:
	xorq	(%r11), %r13
	movq	8(%r11), %rax
	xorq	%rcx, %rax
	mulq	%r13
	movq	%rdx, %r13
	xorq	%rax, %r13
	xorq	16(%r11), %rsi
	movq	24(%r11), %rax
	addq	$32, %r11
	xorq	%rcx, %rax
	mulq	%rsi
	movq	%rdx, %rsi
	xorq	%rax, %rsi
	addq	$-2, %r8
	jne	.LBB21_22
.LBB21_23:
	testb	$16, %dil
	jne	.LBB21_25
	xorq	(%r11), %r13
	movq	8(%r11), %rax
	xorq	%rcx, %rax
	mulq	%r13
	movq	%rsi, %r13
	movq	%rdx, %rsi
	xorq	%rax, %rsi
.LBB21_25:
	xorq	-16(%r10), %r13
	xorq	-8(%r10), %rsi
	jmp	.LBB21_32
.LBB21_42:
	movq	%r15, %r8
	movq	%r12, %rdi
	movq	16(%rsp), %rcx
	cmpq	%r9, %rcx
	je	.LBB21_47
	movzbl	(%r10), %edi
	movq	40(%rsp), %rax
	shrq	%rax
	movzbl	(%r10,%rax), %eax
	movzbl	-1(%rbp,%rcx), %r8d
	xorq	%r12, %rdi
	shll	$8, %r8d
	orq	%rax, %r8
.LBB21_46:
	xorq	%r15, %r8
.LBB21_47:
	movabsq	$-1065810590584100411, %r10
	movq	%r9, %r11
	imulq	%r10, %r11
	movq	%r13, %rax
	mulq	%rsi
	movq	%rdx, %r14
	xorq	%r9, %r14
	xorq	%rax, %r14
	addq	%r11, %r14
	imulq	%r10, %r14
	movq	%rdi, %rax
	mulq	%r8
	movq	40(%rsp), %rsi
	xorq	%rdx, %rsi
	xorq	%rax, %rsi
	addq	%r14, %rsi
	imulq	%r10, %rsi
	rolq	$26, %rsi
	movq	8(%rsp), %rdi
	movq	%rsi, 40(%rsp)
	movq	%rbp, %rdx
	callq	_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E
	testb	$1, %al
	je	.LBB21_48
	movl	%edx, %r14d
	jmp	.LBB21_150
.LBB21_48:
	movq	%rbp, %r13
	xorl	%ebp, %ebp
	movq	8(%rsp), %r10
	movq	16(%rsp), %rsi
	.p2align	4
.LBB21_49:
	cmpq	%rbx, %r13
	je	.LBB21_68
	movsbl	-1(%rbx), %eax
	testl	%eax, %eax
	js	.LBB21_52
	decq	%rbx
	cmpl	$47, %eax
	jne	.LBB21_60
	jmp	.LBB21_61
	.p2align	4
.LBB21_52:
	movzbl	-2(%rbx), %ecx
	cmpb	$-64, %cl
	jge	.LBB21_53
	movzbl	-3(%rbx), %edx
	cmpb	$-64, %dl
	jge	.LBB21_55
	movzbl	-4(%rbx), %edi
	addq	$-4, %rbx
	andl	$7, %edi
	shll	$6, %edi
	andl	$63, %edx
	orl	%edi, %edx
	jmp	.LBB21_57
.LBB21_53:
	addq	$-2, %rbx
	andl	$31, %ecx
	jmp	.LBB21_58
.LBB21_55:
	addq	$-3, %rbx
	andl	$15, %edx
.LBB21_57:
	shll	$6, %edx
	andl	$63, %ecx
	orl	%edx, %ecx
.LBB21_58:
	shll	$6, %ecx
	andl	$63, %eax
	orl	%ecx, %eax
	cmpl	$47, %eax
	je	.LBB21_61
.LBB21_60:
	cmpl	$35, %eax
	jne	.LBB21_49
.LBB21_61:
	subq	%r13, %rbx
	incq	%rbx
	jne	.LBB21_63
	xorl	%ebp, %ebp
	jmp	.LBB21_68
.LBB21_63:
	cmpq	%rsi, %rbx
	jae	.LBB21_64
	cmpb	$-64, (%r13,%rbx)
	jl	.LBB21_65
	movq	%rbx, %rbp
	jmp	.LBB21_68
.LBB21_64:
	movq	%rsi, %rbp
	jne	.LBB21_65
.LBB21_68:
	movabsq	$4919460506697669435, %rbx
	movabsq	$1452335207727870361, %r11
	movq	%r13, %r8
	addq	%rbp, %r8
	cmpq	$0, 144(%r10)
	movq	%rbp, 56(%rsp)
	movq	%r8, 128(%rsp)
	je	.LBB21_93
	cmpq	$17, %rbp
	jae	.LBB21_70
	cmpq	$7, %rbp
	jbe	.LBB21_78
	movq	(%r13), %rcx
	xorq	%r12, %rcx
	movq	-8(%r8), %rsi
	jmp	.LBB21_83
.LBB21_70:
	leaq	-17(%rbp), %rdi
	movq	%r12, %rcx
	movq	%r15, %rsi
	movq	%r13, %r9
	cmpq	$16, %rdi
	jb	.LBB21_74
	movq	%r8, %r14
	movq	%rdi, %r8
	shrq	$4, %r8
	incq	%r8
	andq	$-2, %r8
	movq	%r12, %rcx
	movq	%r15, %rsi
	movq	%r13, %r9
	movabsq	$-6626703657320631856, %r11
	.p2align	4
.LBB21_72:
	xorq	(%r9), %rcx
	movq	8(%r9), %rax
	xorq	%r11, %rax
	mulq	%rcx
	movq	%rdx, %rcx
	xorq	%rax, %rcx
	xorq	16(%r9), %rsi
	movq	24(%r9), %rax
	addq	$32, %r9
	xorq	%r11, %rax
	mulq	%rsi
	movq	%rdx, %rsi
	xorq	%rax, %rsi
	addq	$-2, %r8
	jne	.LBB21_72
	movq	%r14, %r8
	movabsq	$1452335207727870361, %r11
.LBB21_74:
	testb	$16, %dil
	jne	.LBB21_76
	xorq	(%r9), %rcx
	movq	8(%r9), %rax
	movabsq	$-6626703657320631856, %rdx
	xorq	%rdx, %rax
	mulq	%rcx
	movq	%rsi, %rcx
	movq	%rdx, %rsi
	xorq	%rax, %rsi
.LBB21_76:
	xorq	-16(%r8), %rcx
	xorq	-8(%r8), %rsi
	jmp	.LBB21_84
.LBB21_27:
	cmpq	$3, %r9
	jbe	.LBB21_28
	movq	%rsi, %rbp
	movl	(%rsi), %r13d
	movl	-4(%r10), %esi
	xorq	%r12, %r13
	jmp	.LBB21_31
.LBB21_78:
	cmpq	$3, %rbp
	jbe	.LBB21_79
	movl	(%r13), %ecx
	movl	-4(%r8), %esi
	xorq	%r12, %rcx
	jmp	.LBB21_83
.LBB21_28:
	movzbl	(%rsi), %r13d
	movq	%r9, %rax
	shrq	%rax
	movzbl	(%rsi,%rax), %eax
	movq	%rsi, %rbp
	movzbl	-1(%rsi,%r9), %esi
	xorq	%r12, %r13
	shll	$8, %esi
	orq	%rax, %rsi
.LBB21_31:
	xorq	%r15, %rsi
	jmp	.LBB21_32
.LBB21_79:
	movq	%r15, %rsi
	movq	%r12, %rcx
	testq	%rbp, %rbp
	je	.LBB21_84
	movzbl	(%r13), %ecx
	movq	%rbp, %rax
	shrq	%rax
	movzbl	(%r13,%rax), %eax
	movzbl	-1(%r13,%rbp), %esi
	xorq	%r12, %rcx
	shll	$8, %esi
	orq	%rax, %rsi
.LBB21_83:
	xorq	%r15, %rsi
.LBB21_84:
	movq	%rcx, %rax
	mulq	%rsi
	xorq	%rbp, %rdx
	xorq	%rax, %rdx
	imulq	%r11, %rdx
	addq	%rbx, %rdx
	rolq	$26, %rdx
	movq	%rdx, %rax
	shrq	$57, %rax
	movq	120(%r10), %rcx
	movq	128(%r10), %rsi
	movd	%eax, %xmm0
	punpcklbw	%xmm0, %xmm0
	pshuflw	$0, %xmm0, %xmm0
	pshufd	$0, %xmm0, %xmm1
	xorl	%edi, %edi
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r14
.LBB21_85:
	andq	%rsi, %rdx
	movdqu	(%rcx,%rdx), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %eax
	testl	%eax, %eax
	je	.LBB21_90
	movq	%rdx, 32(%rsp)
	movq	%rsi, 48(%rsp)
	movdqa	%xmm1, 96(%rsp)
	movq	%rdi, 80(%rsp)
	movdqa	%xmm3, 64(%rsp)
.LBB21_87:
	movq	%rax, 24(%rsp)
	rep		bsfl	%eax, %eax
	addq	%rdx, %rax
	andq	%rsi, %rax
	negq	%rax
	leaq	(%rax,%rax,2), %rax
	cmpq	-16(%rcx,%rax,8), %rbp
	jne	.LBB21_89
	leaq	(%rcx,%rax,8), %rbx
	movq	-24(%rbx), %rsi
	movq	%r13, %rdi
	movq	%rbp, %rdx
	movq	%rcx, %rbp
	callq	*%r14
	movq	%rbp, %rcx
	movq	56(%rsp), %rbp
	movq	8(%rsp), %r10
	testl	%eax, %eax
	je	.LBB21_92
.LBB21_89:
	movq	24(%rsp), %rdx
	leal	-1(%rdx), %eax
	andw	%dx, %ax
	movq	32(%rsp), %rdx
	movq	48(%rsp), %rsi
	movdqa	96(%rsp), %xmm1
	movq	80(%rsp), %rdi
	pcmpeqd	%xmm2, %xmm2
	movdqa	64(%rsp), %xmm3
	jne	.LBB21_87
.LBB21_90:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB21_93
	addq	%rdi, %rdx
	addq	$16, %rdx
	addq	$16, %rdi
	jmp	.LBB21_85
.LBB21_93:
	testq	%rbp, %rbp
	js	.LBB21_153
	movq	16(%r10), %rax
	movq	%rax, 24(%rsp)
	je	.LBB21_95
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%rbp, %rdi
	callq	*malloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB21_154
	movq	%rax, %rdi
	movq	%r13, %rsi
	movq	%rbp, %rdx
	movq	%rax, %r14
	callq	*memcpy@GOTPCREL(%rip)
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%rbp, %rdi
	callq	*malloc@GOTPCREL(%rip)
	movq	%r14, %rsi
	movq	%rax, %r14
	testq	%rax, %rax
	jne	.LBB21_98
.LBB21_154:
	movl	$1, %edi
	movq	%rbp, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB21_92:
	movl	-8(%rbx), %eax
	movq	%rax, 24(%rsp)
	movq	16(%rsp), %r14
	subq	%rbp, %r14
	jns	.LBB21_135
	jmp	.LBB21_153
.LBB21_95:
	movl	$1, %esi
	movl	$1, %r14d
.LBB21_98:
	movq	%r14, %rdi
	movq	%rsi, 32(%rsp)
	movq	%rbp, %rdx
	callq	*memcpy@GOTPCREL(%rip)
	movq	8(%rsp), %r8
	movq	24(%rsp), %rax
	cmpq	(%r8), %rax
	jne	.LBB21_100
	movq	%r8, %rdi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h0bad941ba750e446E
	movq	8(%rsp), %r8
.LBB21_100:
	movq	8(%r8), %rax
	movq	24(%rsp), %rdx
	movq	%rdx, %rcx
	shlq	$4, %rcx
	movq	%r14, (%rax,%rcx)
	movq	%rbp, 8(%rax,%rcx)
	leaq	1(%rdx), %rax
	movq	%rax, 16(%r8)
	cmpq	$17, %rbp
	jae	.LBB21_101
	cmpq	$7, %rbp
	movabsq	$4919460506697669435, %r10
	movq	32(%rsp), %rdi
	jbe	.LBB21_110
	xorq	(%rdi), %r12
	xorq	-8(%rdi,%rbp), %r15
	jmp	.LBB21_115
.LBB21_101:
	leaq	-17(%rbp), %rcx
	cmpq	$16, %rcx
	movabsq	$4919460506697669435, %r10
	movq	32(%rsp), %rdi
	jae	.LBB21_103
	movq	%rdi, %rsi
	testb	$16, %cl
	je	.LBB21_107
	jmp	.LBB21_108
.LBB21_110:
	cmpq	$3, %rbp
	jbe	.LBB21_111
	movl	(%rdi), %eax
	movl	-4(%rdi,%rbp), %ecx
	xorq	%rax, %r12
	xorq	%rcx, %r15
	jmp	.LBB21_115
.LBB21_103:
	movq	%rcx, %rsi
	shrq	$4, %rsi
	incq	%rsi
	andq	$-2, %rsi
	movabsq	$-6626703657320631856, %r9
	.p2align	4
.LBB21_104:
	xorq	(%rdi), %r12
	movq	8(%rdi), %rax
	xorq	%r9, %rax
	mulq	%r12
	movq	%rdx, %r12
	xorq	%rax, %r12
	xorq	16(%rdi), %r15
	movq	24(%rdi), %rax
	addq	$32, %rdi
	xorq	%r9, %rax
	mulq	%r15
	movq	%rdx, %r15
	xorq	%rax, %r15
	addq	$-2, %rsi
	jne	.LBB21_104
	movq	32(%rsp), %rsi
	testb	$16, %cl
	jne	.LBB21_108
.LBB21_107:
	xorq	(%rdi), %r12
	movabsq	$-6626703657320631856, %rax
	xorq	8(%rdi), %rax
	mulq	%r12
	movq	%r15, %r12
	movq	%rdx, %r15
	xorq	%rax, %r15
.LBB21_108:
	movq	%rsi, %rdi
	xorq	-16(%rsi,%rbp), %r12
	xorq	-8(%rsi,%rbp), %r15
.LBB21_115:
	movq	%r12, %rax
	mulq	%r15
	xorq	%rbp, %rdx
	xorq	%rax, %rdx
	movabsq	$1452335207727870361, %rax
	imulq	%rax, %rdx
	addq	%r10, %rdx
	rolq	$26, %rdx
	cmpq	$0, 136(%r8)
	je	.LBB21_116
.LBB21_117:
	movq	120(%r8), %r14
	movq	128(%r8), %rcx
	movq	%rdx, %r15
	shrq	$57, %r15
	movd	%r15d, %xmm0
	punpcklbw	%xmm0, %xmm0
	pshuflw	$0, %xmm0, %xmm0
	pshufd	$0, %xmm0, %xmm1
	xorl	%esi, %esi
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r13
	xorl	%r10d, %r10d
.LBB21_118:
	andq	%rcx, %rdx
	movdqu	(%r14,%rdx), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r12d
	testl	%r12d, %r12d
	je	.LBB21_123
	movq	%rdx, 48(%rsp)
	movq	%rcx, 96(%rsp)
	movdqa	%xmm1, 80(%rsp)
	movq	%rsi, 64(%rsp)
	movq	%r10, 120(%rsp)
	movdqa	%xmm3, 192(%rsp)
.LBB21_120:
	rep		bsfl	%r12d, %eax
	addq	%rdx, %rax
	andq	%rcx, %rax
	negq	%rax
	leaq	(%rax,%rax,2), %rax
	movq	56(%rsp), %rbp
	cmpq	-16(%r14,%rax,8), %rbp
	jne	.LBB21_122
	leaq	(%r14,%rax,8), %rbx
	movq	-24(%rbx), %rsi
	movq	%rbp, %rdx
	callq	*%r13
	movq	32(%rsp), %rdi
	movq	8(%rsp), %r8
	testl	%eax, %eax
	je	.LBB21_132
.LBB21_122:
	leal	-1(%r12), %eax
	andw	%r12w, %ax
	movl	%eax, %r12d
	movq	48(%rsp), %rdx
	movq	96(%rsp), %rcx
	movdqa	80(%rsp), %xmm1
	movq	64(%rsp), %rsi
	pcmpeqd	%xmm2, %xmm2
	movq	120(%rsp), %r10
	movdqa	192(%rsp), %xmm3
	jne	.LBB21_120
.LBB21_123:
	cmpq	$1, %r10
	movq	56(%rsp), %rbp
	movq	112(%rsp), %r10
	je	.LBB21_126
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	je	.LBB21_155
	rep		bsfl	%eax, %r10d
	addq	%rdx, %r10
	andq	%rcx, %r10
.LBB21_126:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB21_129
	movq	%r10, 112(%rsp)
	movl	$1, %r10d
	jmp	.LBB21_128
.LBB21_155:
	xorl	%r10d, %r10d
.LBB21_128:
	addq	%rsi, %rdx
	addq	$16, %rdx
	addq	$16, %rsi
	jmp	.LBB21_118
.LBB21_132:
	movq	24(%rsp), %rax
	movl	%eax, -8(%rbx)
	testq	%rbp, %rbp
	je	.LBB21_134
	callq	*free@GOTPCREL(%rip)
.LBB21_134:
	movq	16(%rsp), %r14
	subq	%rbp, %r14
	js	.LBB21_153
.LBB21_135:
	movl	$1, %ebx
	je	.LBB21_138
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r14, %rdi
	callq	*malloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB21_156
	movq	%rax, %rbx
.LBB21_138:
	movq	%rbx, %rdi
	movq	128(%rsp), %rsi
	movq	%r14, %r13
	movq	%r14, %rdx
	callq	*memcpy@GOTPCREL(%rip)
	movq	8(%rsp), %r8
	movq	64(%r8), %r12
	movl	216(%r8), %r14d
	addl	%r12d, %r14d
	incl	%r14d
	js	.LBB21_157
	leaq	48(%r8), %r15
	cmpq	48(%r8), %r12
	jne	.LBB21_141
	movq	%r15, %rdi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hb1fca42c238c024cE
	movq	8(%rsp), %r8
.LBB21_141:
	movq	56(%r8), %rax
	leaq	(%r12,%r12,4), %rcx
	movl	$0, (%rax,%rcx,8)
	movq	24(%rsp), %rdx
	movl	%edx, 4(%rax,%rcx,8)
	movq	%rbx, 8(%rax,%rcx,8)
	movq	%r13, 16(%rax,%rcx,8)
	incq	%r12
	movq	%r12, 64(%r8)
	movq	216(%r8), %rax
	movq	%rax, 136(%rsp)
	movq	184(%r8), %rbx
	movq	192(%r8), %rcx
	movq	%rcx, %rdx
	movq	40(%rsp), %r9
	andq	%r9, %rdx
	movdqu	(%rbx,%rdx), %xmm0
	pmovmskb	%xmm0, %eax
	testl	%eax, %eax
	je	.LBB21_142
.LBB21_144:
	rep		bsfl	%eax, %eax
	addq	%rdx, %rax
	andq	%rcx, %rax
	movzbl	(%rbx,%rax), %esi
	testb	%sil, %sil
	jns	.LBB21_145
.LBB21_146:
	movq	200(%r8), %rdx
	testq	%rdx, %rdx
	sete	%dil
	andb	$1, %sil
	testb	%dil, %sil
	jne	.LBB21_148
	shrq	$57, %r9
	movzbl	%sil, %esi
	subq	%rsi, %rdx
	leaq	-16(%rax), %rsi
	andq	%rcx, %rsi
.LBB21_149:
	movb	%r9b, (%rbx,%rax)
	addq	%rbx, %rsi
	movq	%rdx, 200(%r8)
	movb	%r9b, 16(%rsi)
	incq	208(%r8)
	shlq	$2, %rax
	subq	%rax, %rbx
	movl	%r14d, -4(%rbx)
.LBB21_150:
	movl	%r14d, %eax
	addq	$216, %rsp
	.cfi_def_cfa_offset 56
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%r12
	.cfi_def_cfa_offset 40
	popq	%r13
	.cfi_def_cfa_offset 32
	popq	%r14
	.cfi_def_cfa_offset 24
	popq	%r15
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	retq
.LBB21_129:
	.cfi_def_cfa_offset 272
	movzbl	(%r14,%r10), %eax
	testb	%al, %al
	jns	.LBB21_130
.LBB21_131:
	andb	$1, %al
	movzbl	%al, %eax
	subq	%rax, 136(%r8)
	leaq	-16(%r10), %rax
	andq	%rcx, %rax
	movb	%r15b, (%r14,%r10)
	movb	%r15b, 16(%r14,%rax)
	incq	144(%r8)
	negq	%r10
	leaq	(%r10,%r10,2), %rax
	movq	%rdi, -24(%r14,%rax,8)
	movq	%rbp, -16(%r14,%rax,8)
	movq	24(%rsp), %rcx
	movl	%ecx, -8(%r14,%rax,8)
	movq	16(%rsp), %r14
	subq	%rbp, %r14
	jns	.LBB21_135
.LBB21_153:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.1(%rip), %rdi
	callq	_ZN5alloc7raw_vec17capacity_overflow17h1b4b301db4b7931fE
.LBB21_111:
	testq	%rbp, %rbp
	je	.LBB21_115
	movzbl	(%rdi), %eax
	movq	%rbp, %rcx
	shrq	%rcx
	movzbl	(%rdi,%rcx), %ecx
	movzbl	-1(%rdi,%rbp), %edx
	xorq	%rax, %r12
	shll	$8, %edx
	orq	%rcx, %rdx
	xorq	%rdx, %r15
	jmp	.LBB21_115
.LBB21_142:
	movl	$16, %esi
.LBB21_143:
	addq	%rsi, %rdx
	andq	%rcx, %rdx
	movdqu	(%rbx,%rdx), %xmm0
	pmovmskb	%xmm0, %eax
	addq	$16, %rsi
	testl	%eax, %eax
	jne	.LBB21_144
	jmp	.LBB21_143
.LBB21_157:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.17(%rip), %rax
	movq	%rax, 144(%rsp)
	movq	$1, 152(%rsp)
	movq	$8, 160(%rsp)
	pxor	%xmm0, %xmm0
	movdqu	%xmm0, 168(%rsp)
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.18(%rip), %rsi
	leaq	144(%rsp), %rdi
	callq	_ZN4core9panicking9panic_fmt17hc8737e8cca20a7c8E
.LBB21_145:
	movdqa	(%rbx), %xmm0
	pmovmskb	%xmm0, %eax
	rep		bsfl	%eax, %eax
	movzbl	(%rbx,%rax), %esi
	jmp	.LBB21_146
.LBB21_116:
	leaq	120(%r8), %rdi
	leaq	152(%r8), %rsi
	movq	%rdx, %rbx
	callq	_ZN9hashbrown3raw21RawTable$LT$T$C$A$GT$14reserve_rehash17h9e8a4792566125f8E
	movq	32(%rsp), %rdi
	movq	8(%rsp), %r8
	movq	%rbx, %rdx
	jmp	.LBB21_117
.LBB21_148:
	leaq	72(%r8), %rax
	leaq	24(%r8), %rcx
	leaq	136(%rsp), %rdx
	movq	%rdx, 144(%rsp)
	movq	%r15, 152(%rsp)
	movq	%rax, 160(%rsp)
	movq	%r8, 168(%rsp)
	movq	%rcx, 176(%rsp)
	leaq	184(%r8), %rdi
	leaq	144(%rsp), %rsi
	callq	_ZN9hashbrown3raw21RawTable$LT$T$C$A$GT$14reserve_rehash17hcc79d57eddd45dd4E
	movq	8(%rsp), %rax
	movq	184(%rax), %rbx
	movq	8(%rsp), %rax
	movq	192(%rax), %r15
	movq	%rbx, %rdi
	movq	%r15, %rsi
	movq	40(%rsp), %rdx
	callq	_ZN9hashbrown3raw13RawTableInner17find_insert_index17h02d85789b249c6e3E
	movq	40(%rsp), %r9
	movq	8(%rsp), %r8
	shrq	$57, %r9
	movzbl	(%rbx,%rax), %ecx
	andl	$1, %ecx
	movq	200(%r8), %rdx
	subq	%rcx, %rdx
	leaq	-16(%rax), %rsi
	andq	%r15, %rsi
	jmp	.LBB21_149
.LBB21_130:
	movdqa	(%r14), %xmm0
	pmovmskb	%xmm0, %eax
	rep		bsfl	%eax, %r10d
	movzbl	(%r14,%r10), %eax
	jmp	.LBB21_131
.LBB21_16:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.151(%rip), %r8
	movq	%rsi, %rdi
	movq	%rdx, %rsi
	xorl	%edx, %edx
	callq	_ZN4core3str16slice_error_fail17h9f974238edffa500E
.LBB21_156:
	movl	$1, %edi
	movq	%r14, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB21_65:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.151(%rip), %r8
	movq	%r13, %rdi
	xorl	%edx, %edx
	movq	%rbx, %rcx
	callq	_ZN4core3str16slice_error_fail17h9f974238edffa500E
.Lfunc_end21:
	.size	audit_intern_iri, .Lfunc_end21-audit_intern_iri
