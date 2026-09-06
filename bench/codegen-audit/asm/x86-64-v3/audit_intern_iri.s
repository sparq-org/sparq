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
	subq	$200, %rsp
	.cfi_def_cfa_offset 256
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%rdx, %r14
	movq	%rsi, %r11
	movabsq	$-6626703657320631856, %rbx
	movabsq	$2611923443488327891, %r15
	movabsq	$1376283091369227076, %r12
	leaq	(%rsi,%rdx), %rbp
	movq	%rbp, %rcx
	movq	%rdx, 32(%rsp)
	movq	%rsi, 16(%rsp)
	movq	%rdi, 8(%rsp)
	.p2align	4
.LBB21_1:
	cmpq	%rcx, %r11
	je	.LBB21_156
	movsbl	-1(%rcx), %eax
	testl	%eax, %eax
	js	.LBB21_4
	decq	%rcx
	cmpl	$47, %eax
	jne	.LBB21_12
	jmp	.LBB21_13
	.p2align	4
.LBB21_4:
	movzbl	-2(%rcx), %edx
	cmpb	$-64, %dl
	jge	.LBB21_5
	movzbl	-3(%rcx), %esi
	cmpb	$-64, %sil
	jge	.LBB21_7
	movzbl	-4(%rcx), %r8d
	addq	$-4, %rcx
	andl	$7, %r8d
	shll	$6, %r8d
	andl	$63, %esi
	orl	%r8d, %esi
	jmp	.LBB21_9
.LBB21_5:
	addq	$-2, %rcx
	andl	$31, %edx
	jmp	.LBB21_10
.LBB21_7:
	addq	$-3, %rcx
	andl	$15, %esi
.LBB21_9:
	shll	$6, %esi
	andl	$63, %edx
	orl	%esi, %edx
.LBB21_10:
	shll	$6, %edx
	andl	$63, %eax
	orl	%edx, %eax
	cmpl	$47, %eax
	je	.LBB21_13
.LBB21_12:
	cmpl	$35, %eax
	jne	.LBB21_1
.LBB21_13:
	subq	%r11, %rcx
	incq	%rcx
	jne	.LBB21_14
.LBB21_156:
	xorl	%r13d, %r13d
	movq	%r14, %rsi
	movq	%r11, %r9
	movq	%r12, %r8
	movq	%r15, %rax
.LBB21_33:
	cmpq	$17, %rsi
	movq	%r13, 24(%rsp)
	jae	.LBB21_34
	cmpq	$7, %rsi
	jbe	.LBB21_43
	movq	(%r9), %rcx
	xorq	%r15, %rcx
	movq	-8(%r11,%r14), %r10
	jmp	.LBB21_48
.LBB21_34:
	leaq	-17(%rsi), %rdx
	movq	%rdx, %rcx
	shrq	$4, %rcx
	incq	%rcx
	movl	%ecx, %r11d
	andl	$3, %r11d
	cmpq	$48, %rdx
	jae	.LBB21_36
	movq	%r15, %r14
	movq	%r12, %r10
	jmp	.LBB21_38
.LBB21_14:
	cmpq	%r14, %rcx
	jae	.LBB21_15
	cmpb	$-65, (%r11,%rcx)
	jle	.LBB21_16
	movq	%rcx, %r13
	jmp	.LBB21_19
.LBB21_43:
	cmpq	$3, %rsi
	jbe	.LBB21_44
	movl	(%r9), %ecx
	movl	-4(%r11,%r14), %r10d
	xorq	%r15, %rcx
	jmp	.LBB21_48
.LBB21_36:
	andq	$-4, %rcx
	movq	%r15, %r14
	movq	%r12, %r10
	.p2align	4
.LBB21_37:
	xorq	(%r9), %r14
	movq	8(%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r14, %rdi, %r14
	xorq	16(%r9), %r10
	movq	24(%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %r10, %r13
	xorq	%rdi, %r14
	xorq	32(%r9), %r14
	movq	40(%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r14, %rdi, %r14
	xorq	%r10, %r13
	xorq	48(%r9), %r13
	movq	56(%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r13, %rdx, %r10
	xorq	%rdi, %r14
	addq	$64, %r9
	xorq	%rdx, %r10
	addq	$-4, %rcx
	jne	.LBB21_37
.LBB21_38:
	movq	%r14, %rcx
	testq	%r11, %r11
	je	.LBB21_41
	shll	$4, %r11d
	xorl	%edi, %edi
	.p2align	4
.LBB21_40:
	movq	%r10, %rcx
	xorq	(%r9,%rdi), %r14
	movq	8(%r9,%rdi), %rdx
	xorq	%rbx, %rdx
	mulxq	%r14, %rdx, %r10
	xorq	%rdx, %r10
	addq	$16, %rdi
	movq	%rcx, %r14
	cmpq	%rdi, %r11
	jne	.LBB21_40
.LBB21_41:
	movq	32(%rsp), %r14
	movq	16(%rsp), %r11
	xorq	-16(%r11,%r14), %rcx
	xorq	-8(%r11,%r14), %r10
	jmp	.LBB21_49
.LBB21_15:
	movq	%r14, %r13
	jne	.LBB21_16
.LBB21_19:
	leaq	(%r11,%r13), %r9
	movq	%r14, %rsi
	subq	%r13, %rsi
	cmpq	$17, %r13
	jae	.LBB21_20
	cmpq	$7, %r13
	jbe	.LBB21_28
	movq	(%r11), %rax
	xorq	%r15, %rax
	movq	-8(%r9), %r8
	jmp	.LBB21_32
.LBB21_20:
	leaq	-17(%r13), %rdx
	movq	%rdx, %rax
	shrq	$4, %rax
	incq	%rax
	movl	%eax, %ecx
	andl	$3, %ecx
	movq	%r15, %r10
	movq	%r12, %r8
	cmpq	$48, %rdx
	jb	.LBB21_23
	andq	$-4, %rax
	movq	%r15, %r10
	movq	%r12, %r8
	movq	16(%rsp), %r11
	.p2align	4
.LBB21_22:
	xorq	(%r11), %r10
	movq	8(%r11), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %rdi, %r10
	xorq	16(%r11), %r8
	movq	24(%r11), %rdx
	xorq	%rbx, %rdx
	mulxq	%r8, %r8, %r14
	xorq	%rdi, %r10
	xorq	32(%r11), %r10
	movq	40(%r11), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %rdi, %r10
	xorq	%r8, %r14
	xorq	48(%r11), %r14
	movq	56(%r11), %rdx
	xorq	%rbx, %rdx
	mulxq	%r14, %rdx, %r8
	xorq	%rdi, %r10
	addq	$64, %r11
	xorq	%rdx, %r8
	addq	$-4, %rax
	jne	.LBB21_22
.LBB21_23:
	movq	%r10, %rax
	testq	%rcx, %rcx
	je	.LBB21_26
	shll	$4, %ecx
	xorl	%r14d, %r14d
	.p2align	4
.LBB21_25:
	movq	%r8, %rax
	xorq	(%r11,%r14), %r10
	movq	8(%r11,%r14), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %rdx, %r8
	xorq	%rdx, %r8
	addq	$16, %r14
	movq	%rax, %r10
	cmpq	%r14, %rcx
	jne	.LBB21_25
.LBB21_26:
	xorq	-16(%r9), %rax
	xorq	-8(%r9), %r8
	movq	32(%rsp), %r14
	movq	16(%rsp), %r11
	jmp	.LBB21_33
.LBB21_44:
	movq	%r12, %r10
	movq	%r15, %rcx
	cmpq	%r13, %r14
	je	.LBB21_49
	movzbl	(%r9), %ecx
	movq	%rsi, %rdx
	shrq	%rdx
	movzbl	(%r9,%rdx), %edx
	movzbl	-1(%r11,%r14), %r10d
	xorq	%r15, %rcx
	shll	$8, %r10d
	orq	%rdx, %r10
.LBB21_48:
	xorq	%r12, %r10
.LBB21_49:
	movabsq	$-1065810590584100411, %rdi
	movq	24(%rsp), %r13
	movq	%r13, %r9
	imulq	%rdi, %r9
	movq	%rax, %rdx
	mulxq	%r8, %r8, %rax
	xorq	%r13, %r8
	xorq	%rax, %r8
	addq	%r9, %r8
	imulq	%rdi, %r8
	movq	%rcx, %rdx
	mulxq	%r10, %rcx, %rax
	xorq	%rcx, %rsi
	xorq	%rax, %rsi
	addq	%r8, %rsi
	imulq	%rdi, %rsi
	rorxq	$38, %rsi, %rsi
	movq	8(%rsp), %rdi
	movq	%rsi, 136(%rsp)
	movq	%r11, %rdx
	movq	%r14, %rcx
	callq	_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E
	testb	$1, %al
	jne	.LBB21_155
	xorl	%r10d, %r10d
	movq	16(%rsp), %rdi
	.p2align	4
.LBB21_51:
	cmpq	%rbp, %rdi
	je	.LBB21_70
	movsbl	-1(%rbp), %eax
	testl	%eax, %eax
	js	.LBB21_54
	decq	%rbp
	cmpl	$47, %eax
	jne	.LBB21_62
	jmp	.LBB21_63
	.p2align	4
.LBB21_54:
	movzbl	-2(%rbp), %ecx
	cmpb	$-64, %cl
	jge	.LBB21_55
	movzbl	-3(%rbp), %edx
	cmpb	$-64, %dl
	jge	.LBB21_57
	movzbl	-4(%rbp), %esi
	addq	$-4, %rbp
	andl	$7, %esi
	shll	$6, %esi
	andl	$63, %edx
	orl	%esi, %edx
	jmp	.LBB21_59
.LBB21_55:
	addq	$-2, %rbp
	andl	$31, %ecx
	jmp	.LBB21_60
.LBB21_57:
	addq	$-3, %rbp
	andl	$15, %edx
.LBB21_59:
	shll	$6, %edx
	andl	$63, %ecx
	orl	%edx, %ecx
.LBB21_60:
	shll	$6, %ecx
	andl	$63, %eax
	orl	%ecx, %eax
	cmpl	$47, %eax
	je	.LBB21_63
.LBB21_62:
	cmpl	$35, %eax
	jne	.LBB21_51
.LBB21_63:
	subq	%rdi, %rbp
	incq	%rbp
	jne	.LBB21_65
	xorl	%r10d, %r10d
	jmp	.LBB21_70
.LBB21_65:
	cmpq	%r14, %rbp
	jae	.LBB21_66
	cmpb	$-64, (%rdi,%rbp)
	jl	.LBB21_67
	movq	%rbp, %r10
	jmp	.LBB21_70
.LBB21_66:
	movq	%r14, %r10
	jne	.LBB21_67
.LBB21_70:
	leaq	(%rdi,%r10), %r14
	movq	8(%rsp), %rax
	cmpq	$0, 144(%rax)
	movq	%r10, 24(%rsp)
	movq	%r14, 128(%rsp)
	je	.LBB21_96
	cmpq	$17, %r10
	jae	.LBB21_72
	cmpq	$7, %r10
	jbe	.LBB21_81
	movq	(%rdi), %rax
	xorq	%r15, %rax
	movq	-8(%r14), %rcx
	jmp	.LBB21_86
.LBB21_72:
	leaq	-17(%r10), %rdx
	movq	%rdx, %rax
	shrq	$4, %rax
	incq	%rax
	movl	%eax, %esi
	andl	$3, %esi
	movq	%r15, %r11
	movq	%r12, %rcx
	movq	%rdi, %r8
	cmpq	$48, %rdx
	jb	.LBB21_76
	andq	$-4, %rax
	movq	%r15, %r11
	movq	%r12, %rcx
	movq	%rdi, %r8
	.p2align	4
.LBB21_74:
	xorq	(%r8), %r11
	movq	8(%r8), %rdx
	xorq	%rbx, %rdx
	mulxq	%r11, %r11, %r9
	xorq	16(%r8), %rcx
	movq	24(%r8), %rdx
	xorq	%rbx, %rdx
	mulxq	%rcx, %rcx, %r10
	xorq	%r11, %r9
	xorq	32(%r8), %r9
	movq	40(%r8), %rdx
	xorq	%rbx, %rdx
	mulxq	%r9, %r9, %r11
	xorq	%rcx, %r10
	xorq	48(%r8), %r10
	movq	56(%r8), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %rdx, %rcx
	xorq	%r9, %r11
	addq	$64, %r8
	xorq	%rdx, %rcx
	addq	$-4, %rax
	jne	.LBB21_74
	movq	24(%rsp), %r10
.LBB21_76:
	movq	%r11, %rax
	testq	%rsi, %rsi
	je	.LBB21_79
	shll	$4, %esi
	xorl	%r9d, %r9d
	.p2align	4
.LBB21_78:
	movq	%rcx, %rax
	xorq	(%r8,%r9), %r11
	movq	8(%r8,%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r11, %rdx, %rcx
	xorq	%rdx, %rcx
	addq	$16, %r9
	movq	%rax, %r11
	cmpq	%r9, %rsi
	jne	.LBB21_78
.LBB21_79:
	xorq	-16(%r14), %rax
	xorq	-8(%r14), %rcx
	jmp	.LBB21_87
.LBB21_28:
	cmpq	$3, %r13
	jbe	.LBB21_29
	movl	(%r11), %eax
	movl	-4(%r9), %r8d
	xorq	%r15, %rax
	jmp	.LBB21_32
.LBB21_81:
	cmpq	$3, %r10
	jbe	.LBB21_82
	movl	(%rdi), %eax
	movl	-4(%r14), %ecx
	xorq	%r15, %rax
	jmp	.LBB21_86
.LBB21_29:
	movzbl	(%r11), %eax
	movq	%r13, %rcx
	shrq	%rcx
	movzbl	(%r11,%rcx), %ecx
	movzbl	-1(%r11,%r13), %r8d
	xorq	%r15, %rax
	shll	$8, %r8d
	orq	%rcx, %r8
.LBB21_32:
	xorq	%r12, %r8
	jmp	.LBB21_33
.LBB21_82:
	movq	%r12, %rcx
	movq	%r15, %rax
	testq	%r10, %r10
	je	.LBB21_87
	movzbl	(%rdi), %eax
	movq	%r10, %rcx
	shrq	%rcx
	movzbl	(%rdi,%rcx), %edx
	movzbl	-1(%rdi,%r10), %ecx
	xorq	%r15, %rax
	shll	$8, %ecx
	orq	%rdx, %rcx
.LBB21_86:
	xorq	%r12, %rcx
.LBB21_87:
	movq	%rax, %rdx
	mulxq	%rcx, %rcx, %rax
	xorq	%r10, %rcx
	xorq	%rax, %rcx
	movabsq	$1452335207727870361, %rax
	imulq	%rax, %rcx
	movabsq	$4919460506697669435, %rax
	addq	%rax, %rcx
	rorxq	$38, %rcx, %rbp
	movq	%rbp, %rax
	shrq	$57, %rax
	movq	8(%rsp), %rdx
	movq	120(%rdx), %rcx
	movq	128(%rdx), %rdx
	vmovd	%eax, %xmm0
	vpbroadcastb	%xmm0, %xmm1
	xorl	%esi, %esi
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r13
.LBB21_88:
	andq	%rdx, %rbp
	vmovdqu	(%rcx,%rbp), %xmm3
	vpcmpeqb	%xmm1, %xmm3, %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	je	.LBB21_93
	movq	%rdx, 56(%rsp)
	vmovdqa	%xmm1, 80(%rsp)
	movq	%rsi, 48(%rsp)
	vmovdqa	%xmm3, 64(%rsp)
.LBB21_90:
	movq	%rax, 96(%rsp)
	tzcntl	%eax, %eax
	addq	%rbp, %rax
	andq	%rdx, %rax
	negq	%rax
	leaq	(%rax,%rax,2), %rax
	cmpq	-16(%rcx,%rax,8), %r10
	jne	.LBB21_92
	leaq	(%rcx,%rax,8), %r14
	movq	-24(%r14), %rsi
	movq	%r10, %rdx
	movq	%rcx, 40(%rsp)
	callq	*%r13
	movq	40(%rsp), %rcx
	movq	24(%rsp), %r10
	movq	16(%rsp), %rdi
	testl	%eax, %eax
	je	.LBB21_95
.LBB21_92:
	movq	96(%rsp), %rdx
	leal	-1(%rdx), %eax
	andw	%dx, %ax
	movq	56(%rsp), %rdx
	vmovdqa	80(%rsp), %xmm1
	movq	48(%rsp), %rsi
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	vmovdqa	64(%rsp), %xmm3
	jne	.LBB21_90
.LBB21_93:
	vpcmpeqb	%xmm2, %xmm3, %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	jne	.LBB21_96
	addq	%rsi, %rbp
	addq	$16, %rbp
	addq	$16, %rsi
	jmp	.LBB21_88
.LBB21_96:
	testq	%r10, %r10
	js	.LBB21_157
	movq	8(%rsp), %rax
	movq	16(%rax), %rbp
	movq	%rbp, 40(%rsp)
	je	.LBB21_98
	movq	8(%rsp), %r14
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r10, %rdi
	movq	%r10, %rbp
	callq	*malloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB21_158
	movq	%rax, %rdi
	movq	16(%rsp), %rsi
	movq	%rbp, %rdx
	movq	%rax, %r13
	callq	*memcpy@GOTPCREL(%rip)
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%rbp, %rdi
	callq	*malloc@GOTPCREL(%rip)
	movq	%r13, %rsi
	movq	%rax, %r13
	testq	%rax, %rax
	movq	%rbp, %r10
	movq	40(%rsp), %rbp
	jne	.LBB21_101
	movl	$1, %edi
	movq	%r10, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB21_95:
	movl	-8(%r14), %ebp
	movq	32(%rsp), %r12
	subq	%r10, %r12
	jns	.LBB21_140
	jmp	.LBB21_157
.LBB21_98:
	movq	8(%rsp), %r14
	movl	$1, %esi
	movl	$1, %r13d
.LBB21_101:
	movq	%r13, %rdi
	movq	%rsi, 16(%rsp)
	movq	%r10, %rdx
	callq	*memcpy@GOTPCREL(%rip)
	cmpq	(%r14), %rbp
	jne	.LBB21_103
	movq	%r14, %rdi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h0bad941ba750e446E
.LBB21_103:
	movq	8(%r14), %rax
	movq	%rbp, %rcx
	shlq	$4, %rcx
	movq	%r13, (%rax,%rcx)
	movq	24(%rsp), %r10
	movq	%r10, 8(%rax,%rcx)
	leaq	1(%rbp), %rax
	movq	%rax, 16(%r14)
	cmpq	$17, %r10
	jae	.LBB21_104
	cmpq	$7, %r10
	movq	16(%rsp), %rdi
	jbe	.LBB21_114
	xorq	(%rdi), %r15
	xorq	-8(%rdi,%r10), %r12
	jmp	.LBB21_119
.LBB21_104:
	leaq	-17(%r10), %rdx
	movq	%rdx, %rcx
	shrq	$4, %rcx
	incq	%rcx
	movl	%ecx, %eax
	andl	$3, %eax
	cmpq	$48, %rdx
	movq	16(%rsp), %rdi
	jae	.LBB21_106
	movq	%rdi, %rsi
	movq	%r15, %rcx
	testq	%rax, %rax
	jne	.LBB21_110
	jmp	.LBB21_112
.LBB21_114:
	cmpq	$3, %r10
	jbe	.LBB21_115
	movl	(%rdi), %eax
	movl	-4(%rdi,%r10), %ecx
	xorq	%rax, %r15
	xorq	%rcx, %r12
	jmp	.LBB21_119
.LBB21_106:
	andq	$-4, %rcx
	movq	%rdi, %rsi
	.p2align	4
.LBB21_107:
	xorq	(%rsi), %r15
	movq	8(%rsi), %rdx
	xorq	%rbx, %rdx
	mulxq	%r15, %r11, %r8
	xorq	16(%rsi), %r12
	movq	24(%rsi), %rdx
	xorq	%rbx, %rdx
	mulxq	%r12, %r9, %r10
	xorq	%r11, %r8
	xorq	32(%rsi), %r8
	movq	40(%rsi), %rdx
	xorq	%rbx, %rdx
	mulxq	%r8, %r8, %r15
	xorq	%r9, %r10
	xorq	48(%rsi), %r10
	movq	56(%rsi), %rdx
	xorq	%rbx, %rdx
	mulxq	%r10, %rdx, %r12
	xorq	%r8, %r15
	addq	$64, %rsi
	xorq	%rdx, %r12
	addq	$-4, %rcx
	jne	.LBB21_107
	movq	24(%rsp), %r10
	movq	%r15, %rcx
	testq	%rax, %rax
	je	.LBB21_112
.LBB21_110:
	shll	$4, %eax
	xorl	%r9d, %r9d
	.p2align	4
.LBB21_111:
	movq	%r12, %rcx
	xorq	(%rsi,%r9), %r15
	movq	8(%rsi,%r9), %rdx
	xorq	%rbx, %rdx
	mulxq	%r15, %rdx, %r12
	xorq	%rdx, %r12
	addq	$16, %r9
	movq	%rcx, %r15
	cmpq	%r9, %rax
	jne	.LBB21_111
.LBB21_112:
	xorq	-16(%rdi,%r10), %rcx
	xorq	-8(%rdi,%r10), %r12
	movq	%rcx, %r15
.LBB21_119:
	movq	%r15, %rdx
	mulxq	%r12, %rcx, %rax
	xorq	%r10, %rcx
	xorq	%rax, %rcx
	movabsq	$1452335207727870361, %rax
	imulq	%rax, %rcx
	movabsq	$4919460506697669435, %rax
	addq	%rax, %rcx
	rorxq	$38, %rcx, %r13
	cmpq	$0, 136(%r14)
	je	.LBB21_120
.LBB21_121:
	movq	120(%r14), %rbx
	movq	128(%r14), %r9
	movq	%r13, %r15
	shrq	$57, %r15
	vmovd	%r15d, %xmm0
	vpbroadcastb	%xmm0, %xmm1
	xorl	%esi, %esi
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r8
	xorl	%ecx, %ecx
.LBB21_122:
	andq	%r9, %r13
	vmovdqu	(%rbx,%r13), %xmm3
	vpcmpeqb	%xmm1, %xmm3, %xmm0
	vpmovmskb	%xmm0, %r14d
	testl	%r14d, %r14d
	je	.LBB21_127
	vmovdqa	%xmm1, 96(%rsp)
	movq	%rsi, 56(%rsp)
	movq	%r9, 80(%rsp)
	movq	%rcx, 48(%rsp)
	vmovdqa	%xmm3, 64(%rsp)
.LBB21_124:
	xorl	%eax, %eax
	tzcntl	%r14d, %eax
	addq	%r13, %rax
	andq	%r9, %rax
	negq	%rax
	leaq	(%rax,%rax,2), %rax
	cmpq	-16(%rbx,%rax,8), %r10
	jne	.LBB21_126
	leaq	(%rbx,%rax,8), %rbp
	movq	-24(%rbp), %rsi
	movq	%r10, %rdx
	movq	%r8, %r12
	callq	*%r8
	movq	%r12, %r8
	movq	16(%rsp), %rdi
	movq	24(%rsp), %r10
	testl	%eax, %eax
	je	.LBB21_136
.LBB21_126:
	leal	-1(%r14), %eax
	andw	%r14w, %ax
	movl	%eax, %r14d
	vmovdqa	96(%rsp), %xmm1
	movq	56(%rsp), %rsi
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	movq	80(%rsp), %r9
	movq	48(%rsp), %rcx
	vmovdqa	64(%rsp), %xmm3
	jne	.LBB21_124
.LBB21_127:
	cmpq	$1, %rcx
	movq	32(%rsp), %r12
	movq	40(%rsp), %rbp
	movq	120(%rsp), %rdx
	je	.LBB21_130
	vpmovmskb	%xmm3, %eax
	testl	%eax, %eax
	je	.LBB21_160
	xorl	%edx, %edx
	tzcntl	%eax, %edx
	addq	%r13, %rdx
	andq	%r9, %rdx
.LBB21_130:
	vpcmpeqb	%xmm2, %xmm3, %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	jne	.LBB21_133
	movq	%rdx, 120(%rsp)
	movl	$1, %ecx
	jmp	.LBB21_132
.LBB21_160:
	xorl	%ecx, %ecx
.LBB21_132:
	addq	%rsi, %r13
	addq	$16, %r13
	addq	$16, %rsi
	jmp	.LBB21_122
.LBB21_136:
	movq	40(%rsp), %rax
	movl	%eax, -8(%rbp)
	movq	%rax, %rbp
	testq	%r10, %r10
	je	.LBB21_138
	callq	*free@GOTPCREL(%rip)
	movq	24(%rsp), %r10
.LBB21_138:
	movq	32(%rsp), %r12
	subq	%r10, %r12
	js	.LBB21_157
.LBB21_140:
	movl	$1, %r14d
	je	.LBB21_143
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r12, %rdi
	callq	*malloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB21_161
	movq	%rax, %r14
.LBB21_143:
	movq	%r14, %rdi
	movq	128(%rsp), %rsi
	movq	%r12, %rdx
	callq	*memcpy@GOTPCREL(%rip)
	movq	8(%rsp), %rax
	movq	64(%rax), %rbx
	movl	216(%rax), %edx
	addl	%ebx, %edx
	incl	%edx
	js	.LBB21_162
	movq	8(%rsp), %rax
	leaq	48(%rax), %rdi
	cmpq	48(%rax), %rbx
	jne	.LBB21_146
	movq	%rbp, %r15
	movl	%edx, %ebp
	movq	%rdi, %r13
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17hb1fca42c238c024cE
	movq	%r13, %rdi
	movl	%ebp, %edx
	movq	%r15, %rbp
.LBB21_146:
	movq	8(%rsp), %rsi
	movq	56(%rsi), %rax
	leaq	(%rbx,%rbx,4), %rcx
	movl	$0, (%rax,%rcx,8)
	movl	%ebp, 4(%rax,%rcx,8)
	movq	%r14, 8(%rax,%rcx,8)
	movq	%r12, 16(%rax,%rcx,8)
	incq	%rbx
	movq	%rbx, 64(%rsi)
	movq	216(%rsi), %rax
	movq	%rax, 144(%rsp)
	movq	184(%rsi), %r14
	movq	192(%rsi), %rcx
	movq	%rcx, %rsi
	movq	136(%rsp), %r12
	andq	%r12, %rsi
	vmovdqu	(%r14,%rsi), %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	je	.LBB21_147
.LBB21_149:
	tzcntl	%eax, %eax
	addq	%rsi, %rax
	andq	%rcx, %rax
	movzbl	(%r14,%rax), %r9d
	testb	%r9b, %r9b
	jns	.LBB21_150
.LBB21_151:
	movq	8(%rsp), %rsi
	movq	200(%rsi), %rsi
	testq	%rsi, %rsi
	sete	%r8b
	andb	$1, %r9b
	testb	%r8b, %r9b
	jne	.LBB21_153
	shrq	$57, %r12
	movzbl	%r9b, %edi
	subq	%rdi, %rsi
	leaq	-16(%rax), %rdi
	andq	%rcx, %rdi
.LBB21_154:
	movb	%r12b, (%r14,%rax)
	addq	%r14, %rdi
	movq	8(%rsp), %rcx
	movq	%rsi, 200(%rcx)
	movb	%r12b, 16(%rdi)
	incq	208(%rcx)
	shlq	$2, %rax
	subq	%rax, %r14
	movl	%edx, -4(%r14)
.LBB21_155:
	movl	%edx, %eax
	addq	$200, %rsp
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
.LBB21_133:
	.cfi_def_cfa_offset 256
	movzbl	(%rbx,%rdx), %eax
	testb	%al, %al
	jns	.LBB21_134
.LBB21_135:
	andb	$1, %al
	movzbl	%al, %eax
	movq	8(%rsp), %rsi
	subq	%rax, 136(%rsi)
	leaq	-16(%rdx), %rax
	andq	%r9, %rax
	movb	%r15b, (%rbx,%rdx)
	movb	%r15b, 16(%rbx,%rax)
	incq	144(%rsi)
	negq	%rdx
	leaq	(%rdx,%rdx,2), %rax
	movq	%rdi, -24(%rbx,%rax,8)
	movq	%r10, -16(%rbx,%rax,8)
	movl	%ebp, -8(%rbx,%rax,8)
	subq	%r10, %r12
	jns	.LBB21_140
.LBB21_157:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.1(%rip), %rdi
	callq	_ZN5alloc7raw_vec17capacity_overflow17h1b4b301db4b7931fE
.LBB21_115:
	testq	%r10, %r10
	je	.LBB21_119
	movzbl	(%rdi), %eax
	movq	%r10, %rcx
	shrq	%rcx
	movzbl	(%rdi,%rcx), %ecx
	movzbl	-1(%rdi,%r10), %edx
	xorq	%rax, %r15
	shll	$8, %edx
	orq	%rcx, %rdx
	xorq	%rdx, %r12
	jmp	.LBB21_119
.LBB21_147:
	movl	$16, %r8d
.LBB21_148:
	addq	%r8, %rsi
	andq	%rcx, %rsi
	vmovdqu	(%r14,%rsi), %xmm0
	vpmovmskb	%xmm0, %eax
	addq	$16, %r8
	testl	%eax, %eax
	jne	.LBB21_149
	jmp	.LBB21_148
.LBB21_162:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.17(%rip), %rax
	movq	%rax, 152(%rsp)
	vmovaps	.LCPI21_0(%rip), %ymm0
	vmovups	%ymm0, 160(%rsp)
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.18(%rip), %rsi
	leaq	152(%rsp), %rdi
	vzeroupper
	callq	_ZN4core9panicking9panic_fmt17hc8737e8cca20a7c8E
.LBB21_150:
	vmovdqa	(%r14), %xmm0
	vpmovmskb	%xmm0, %eax
	tzcntl	%eax, %eax
	movzbl	(%r14,%rax), %r9d
	jmp	.LBB21_151
.LBB21_120:
	leaq	120(%r14), %rdi
	leaq	152(%r14), %rsi
	callq	_ZN9hashbrown3raw21RawTable$LT$T$C$A$GT$14reserve_rehash17h9e8a4792566125f8E
	movq	16(%rsp), %rdi
	movq	24(%rsp), %r10
	jmp	.LBB21_121
.LBB21_153:
	movq	8(%rsp), %r13
	leaq	72(%r13), %rax
	leaq	24(%r13), %rcx
	leaq	144(%rsp), %rsi
	movq	%rsi, 152(%rsp)
	movq	%rdi, 160(%rsp)
	movq	%rax, 168(%rsp)
	movq	%r13, 176(%rsp)
	movq	%rcx, 184(%rsp)
	leaq	184(%r13), %rdi
	leaq	152(%rsp), %rsi
	movl	%edx, %ebx
	callq	_ZN9hashbrown3raw21RawTable$LT$T$C$A$GT$14reserve_rehash17hcc79d57eddd45dd4E
	movq	184(%r13), %r14
	movq	192(%r13), %r15
	movq	%r14, %rdi
	movq	%r15, %rsi
	movq	%r12, %rdx
	callq	_ZN9hashbrown3raw13RawTableInner17find_insert_index17h02d85789b249c6e3E
	movl	%ebx, %edx
	shrq	$57, %r12
	movzbl	(%r14,%rax), %ecx
	andl	$1, %ecx
	movq	200(%r13), %rsi
	subq	%rcx, %rsi
	leaq	-16(%rax), %rdi
	andq	%r15, %rdi
	jmp	.LBB21_154
.LBB21_134:
	vmovdqa	(%rbx), %xmm0
	vpmovmskb	%xmm0, %eax
	xorl	%edx, %edx
	tzcntl	%eax, %edx
	movzbl	(%rbx,%rdx), %eax
	jmp	.LBB21_135
.LBB21_16:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.151(%rip), %r8
	movq	%r11, %rdi
	movq	%r14, %rsi
	xorl	%edx, %edx
	callq	_ZN4core3str16slice_error_fail17h9f974238edffa500E
.LBB21_161:
	movl	$1, %edi
	movq	%r12, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB21_158:
	movl	$1, %edi
	movq	%rbp, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB21_67:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.151(%rip), %r8
	movq	%r14, %rsi
	xorl	%edx, %edx
	movq	%rbp, %rcx
	callq	_ZN4core3str16slice_error_fail17h9f974238edffa500E
.Lfunc_end21:
	.size	audit_intern_iri, .Lfunc_end21-audit_intern_iri
