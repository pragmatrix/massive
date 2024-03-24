## 2022

- Rewrote most of [BrainSharper](https://www.brainsharper.com) in Rust (unreleased).

	- Created a gesture detection library based on event history queries (unreleased).

	- Created a Direct3D low-latency rendering backend for [rust-skia](https://github.com/rust-skia/rust-skia) (unreleased).

	- Experimented with a GUI state management paradigm that uses [sum types](https://en.wikipedia.org/wiki/Tagged_union) for modeling the user's state.

- Got [my first patent](https://register.dpma.de/DPMAregister/pat/PatSchrifteneinsicht?docId=DE102018116701A1&page=1&dpi=300&lang=de) granted 🎉 (in German/y).

## 2021

- Created two high performance services that collect and distribute live session data using WebSockets (Rust, Linux, Docker).

- Created a ingestion service that receives data files and stores them in the file system (Rust, Windows).

- Created a [GitOps](https://about.gitlab.com/topics/gitops/) based distribution and deployment system for Docker containers (Shell scripts, docker-compose, GitLab Pipelines).

- Created a Watchdog service for FreeSWITCH channels.
	- Created a first version of a [FreeSWITCH event socket client for Rust](https://github.com/questnet/free-socks).

## 2020

- Created a registration and control service that communicates with a number of clients by WebSocket in ASP.NET and C#.

- Created a system to charge telephone customers through credit card payments.

- Created a cloud based system for broadcast stations to organize and process telephone raffles. Technologies used were Azure, F#, React & [Material UI](https://material-ui.com/).

- Learned the [Kotlin Programming Language](https://kotlinlang.org/) and [Jetpack Compose](https://developer.android.com/jetpack/compose).

## 2019

- Created Emergent, [a Visual Testrunner for Rust](https://github.com/pragmatrix/emergent).

- Wrapped most of the Skia Graphics Library API for Rust. Open Source and actively maintained [on GitHub](https://github.com/rust-skia/rust-skia).

## 2018

- Learned the [Rust Programming Language](https://www.rust-lang.org/).

- Designed an API for routing telephone calls over a WebSocket interface.

- Created a translation tool in F# that extracts text from .NET applications and supports XLIFF & Excel roundtrips and Google based machine translations.

- Built a prototype for a WebRTC / SIP [Click2Call](https://en.wikipedia.org/wiki/Click-to-call) solution in F# and [Fable](http://fable.io/).

## 2017

- With help of the team at [QuestNet GmbH](http://www.questnet.de), I ported most of their interactive voice applications to VoIP and delivered them to production on an infrastructure that processes several millions telephone calls per year.

- I managed and helped to build an iOS application for scanning 3D body parts that are used to manufacture orthoses for [Ortholutions](http://www.ortholutions.com/).

- Personal Research (F#):
	- Created a rope data structure that can be used in Text Editors.
	- Created an experimental 3D Text renderer based on [Multi-Channel Signed Distance Fields](https://github.com/Chlumsky/msdfgen).
	- Built a simple 3D OpenGL Shader Library.
	- I tried to see how it is possible to map relational programming to F#.
	
## 2016

### April

Started a new iteration of my personal [Live Programming](http://en.wikipedia.org/wiki/Interactive_programming) research: LivePipes provides functions that connect to the IDE and deliver feedback while typing.

## 2015

### November

Created [BuildOnSave](https://github.com/pragmatrix/BuildOnSave), a Visual Studio extension that builds the current solution when a file is saved. Also available in the [Visual Studio Gallery](https://visualstudiogallery.msdn.microsoft.com/2b31b977-ffc9-4066-83e8-c5596786acd0).

### August

Created [AutoFluent](https://github.com/pragmatrix/AutoFluent), a generator for fluent C# APIs in F# and released an [AutoFluent package for Xamarin.Forms](https://www.nuget.org/packages/Xamarin.Forms.AutoFluent/).

Created [Windows 10 Dominator](https://github.com/pragmatrix/Dominator), an open source tool that manages the privacy settings of Windows 10.

### June

Created and presented a F# DSL for specifying Interactive Voice Response (IVR) applications, in which parallel and sequential processes can be combined and coordinated based on their lifetimes and results. The demo uses [Asterisk.ARI](https://wiki.asterisk.org/wiki/pages/viewpage.action?pageId=29395573) to accept and process incoming calls.

### May

Evaluated the current state of Microsoft's [CoreCLR](https://github.com/dotnet/coreclr) and related technologies like DNX, HTTP request handling, XML & Json processing, OR/Ms, Configuration, and Serialization, to find out how to build and distribute server applications on Linux with .NET programming languages.

### March

Created [Dialog](https://github.com/pragmatrix/Dialog), an experimental live programming system for building user interfaces in F#.

### February

Ported Facebook's CSS/flexbox layout engine to [C#/.NET](https://github.com/pragmatrix/css-layout), which was later included in the official [Facebook repository](https://github.com/facebook/css-layout).

## 2014

### November

Implemented a generic version of a [Pratt Parser](http://hall.org.ua/halls/wizzard/pdf/Vaughan.Pratt.TDOP.pdf) in F# that is specifically optimized for parsing expressions of pure functional programs (unreleased).

The German [Amiga enthusiast magazine "Amiga Future"](http://www.amigafuture.de/kb.php?mode=article&k=4661) published the freeware versions of Oktalyzer and [RAP!TOP!COP!](http://www.amigafuture.de/asd.php?asd_id=658).

### October

Released Version 2.3 & 2.4 of [LookHere](http://www.lookhere.pro) to the Apple app store.

### September

Created a pure functional text editor component in F# / OpenGL for a live programming IDE (not released). The editor supports most well known keyboard shortcuts and animates all content based on a differential animation algorithm. Test cases are realized with an internal F# DSL and resemble the typing actions of a user.

### July

Released Version 2.1 of [LookHere](http://www.lookhere.pro) to the Apple app store.

### June

For the upcoming Freeware release of [RAP!TOP!COP!](http://www.amigafuture.de/asd.php?asd_id=658), a software for the Amiga, I created in the 90's, I wrote a [small F# tool](https://github.com/pragmatrix/DocuMark) that converts an type setting format, which was used at Markt & Technik, to Markdown.

### May

Created an Internet Explorer BHO extension in C++.

### April

Gave an [interview to the Amiga enthusiast site Obligement](http://obligement.free.fr/articles_traduction/itwsander_en.php) about Oktalyzer (hosted by Guillaume Guittenit).

### March

Released the second version of [LookHere!](http://www.lookhere.pro/) to the Apple app store.

Began developing sensor visualization components for the web. 

### February

Created [Bricks](https://github.com/pragmatrix/Bricks), an alternative computation model, that implements a lazy evaluated, incremental, and stream-processing capable evaluation core.

Began to implement [BricksUI](https://github.com/pragmatrix/BricksUI), a user interface and live programming IDE, based on Bricks.

### January

Created a Firefox extension in TypeScript.

## 2013

### September

Created a prototype of a simple, lazy, pure functional programming language, and an interpreter that is suitable to be used in live programming environment. The grammar is defined and parsed with [ScanRat](https://github.com/pragmatrix/ScanRat).

Created [Projections](https://github.com/pragmatrix/Projections), an experimental F# library to experiment with streams processing in a lazy, dependency tracked environment.

### August

Created [ScanRat](https://github.com/pragmatrix/ScanRat), a PEG parser combinator library for F#.

### July

Created a Chrome extension in TypeScript.

### May

Experimented with a [WebRTC](http://www.webrtc.org/) and TypeScript based file sharing tool for large files:

- Created [Promise](https://github.com/pragmatrix/Promise), a native Promise implementation for TypeScript.
- Created [iwhen](https://github.com/pragmatrix/iwhen), a TypeScript port of a subset of [when.js](https://github.com/cujojs/when), and changed it so that promises are executed immediately, which is a requirement for [IndexedDB](http://www.w3.org/TR/IndexedDB/). 
- Contributed TypeScript type definitions for [jQuery DataTables](https://datatables.net/) to the [DefinitelyTyped project](https://github.com/borisyankov/DefinitelyTyped).

### March

Created [Promise](https://github.com/pragmatrix/Promise), a strongly typed promise library for Typescript.

### February

Created an [MRCP](http://en.wikipedia.org/wiki/Media_Resource_Control_Protocol)/[SIP](http://de.wikipedia.org/wiki/Session_Initiation_Protocol) client that simplifies access to the [Nuance](http://www.nuance.com/) speech server recognition and text to speech features for [Questnet GmbH](http://questnet.de/). Most parsing was implemented by using [Sprache](https://github.com/sprache/Sprache).
 

### January

Created a .NET communication client and WPF based console for [O2](http://www.o2online.de/) that implements a proprietary mixed text / binary protocol and communicates with their server over sockets and TCP/IP. The protocols were implemented with [Faser](https://github.com/pragmatrix/Faser).

## 2012

### December

Created [Faser](https://github.com/pragmatrix/Faser), a format agnostic serialization library for .NET.

Open sourced [Mime.Encoding](https://github.com/pragmatrix/Mime.Encoding), a .NET mime encoding library I developed for [SharedSafe](https://www.sharedsafe.com/).

### November

Released the [first version of LookHere!](http://www.replicator.org/journal/201211021158-lookhere!-is-available-on-itunes).

Created [SiteSharper](https://github.com/pragmatrix/SiteSharper), a static web site generator in .NET that uses a combination of [Markdown](https://daringfireball.net/projects/markdown/) and [Razor templates](http://en.wikipedia.org/wiki/ASP.NET_Razor_view_engine).

Created and released [Share To Desktop](http://www.replicator.org/journal/201212011402-share-to-desktop) and [Share To Folder](http://www.replicator.org/journal/201212041415-share-to-folder), two small Windows 8 applications to the Windows App Store.

Open sourced [JSNet](https://github.com/pragmatrix/JSNet), a experimental PEG parser generator and a JavaScript parser for .NET.

Created [Konstruktor2](https://github.com/pragmatrix/Konstruktor2), a [dependency injector](http://en.wikipedia.org/wiki/Dependency_injection) for .NET. 

### August

Created [Facts](https://github.com/pragmatrix/Facts), an [update-in](http://clojuredocs.org/clojure_core/clojure.core/update-in) implementation for .NET.

### July

Created [CrossUI](https://github.com/pragmatrix/CrossUI), a .NET drawing API, that is focused on instant test feedback. It has implementations for WPF and WinRT and a test-runner that automatically tracks changed .NET assemblies and renders their contained drawings.

### June

Created, but did not finish [Matter](https://github.com/pragmatrix/matter), a prototype of a small, lisp inspired, programming language.

Created, but did not finish [InConcert](https://github.com/pragmatrix/InConcert), a command line tool to synchronize file system folders.

### April

Created [Rulez](https://github.com/pragmatrix/Rulez), a minimalistic rule evaluation engine.

### February

Began developing [LookHere](http://www.lookhere.pro), an iPad application for construction engineers.

## 2011

### August

Created [NChannels](https://github.com/pragmatrix/NChannels), a .NET implementation of Go's channels.

## 2010

Created and released several versions of [BrainSharper](http://www.brainsharper.com/), a concept mapping editor, for which I created

- an [Event Sourcing](http://martinfowler.com/eaaDev/EventSourcing.html) and [CQRS](http://martinfowler.com/bliki/CQRS.html) framework.
- an [Object-relational mapper](http://en.wikipedia.org/wiki/Object-relational_mapping) for [SQLite](https://sqlite.org/).
- an [SQLite](https://sqlite.org/) client for .NET.
- a concept to implement unlimited Undo / Redo based on Event Sourcing.

## 2009

Created and released several version of [SharedSafe](https://www.sharedsafe.com/), a file sharing tool, with full encryption, automatic synchronization, and conflict resolution.

To make SharedSafe possible, I built:

- a fairly complete IMAP client
- a log structured file system
- cryptographic licensing and management tools
- an update server
- an IPC framework
- a Windows Explorer extension
- a test suite based on [Reactive Extensions](https://rx.codeplex.com/). 
- a Windows setup based on WiX
- a configuration user interface with automatic dependency tracking
- a user interface DSL that composes Windows Forms elements into [responsive disclosure](http://designinginterfaces.com/firstedition/index.php?page=Responsive_Disclosure) based user interface dialogs

Founded the company rootloft GmbH.

Customized xtCommerce and built [egreenfee.de](http://www.egreenfee.de/) for a friend:

- built a PDF generator for golf green-fees with ruby
- customized an xtCommerce Paypal module in PHP

## 2008

Created a workflow engine for telephony applications in C# for [Questnet GmbH](http://questnet.de/).

Created a prototype of a photo organization and tagging application in C#/WPF.  

## 2006

Implemented the workflow engine, business logic, and supervisor component for a bank statement printer "KIT" for Tomaxx GmbH.

Created a generic framework for cash machines, named "libfinance".

## 2005

Implemented large parts of the HDi implementation for [Nero](http://www.nero.com/)'s HD DVD player software and public interfaces, which included:

- An [SMIL](http://de.wikipedia.org/wiki/Synchronized_Multimedia_Integration_Language) implementation.
- An incremental, dependency tracking layouting algorithm for HDi's subset of HTML.
- A dependency tracking [XPath](http://de.wikipedia.org/wiki/XPath) evaluator. 

Implemented the workflow engine, business logic, and supervisor component for the cash depositing machine "DPD Einzahler" and "Cashin 2" for Tomaxx GmbH.

Founded Sander & Messmer GbR.

## 2002 - 2005

Created [CloneDVD](http://en.wikipedia.org/wiki/CloneDVD) together with the team at [Elaborate Bytes](https://www.elby.ch/products/clone_dvd/index.html) until version 2.8.5.1. Developed the user interface, the DVD stream processing engine, managed the build process, and coordinated developers and public releases.

## 2001

Created and updated several versions of a C++ based telephony server and TCP/IP protocol implementation that allowed business logic clients to access several [Aculab](http://www.aculab.com/) hardware features in parallel. The server is in use at [Questnet](http://questnet.de/) and powers all telephone calls and services. About 50 million telephone calls in 2013.

## 2000

Created an experimental, small OpenGL based 3D engine in Haskell to see if lazy evaluation can be used to increase performance.

## 1999 - 2000

Contributed to the development of a Internet Casino Client for Modern Technologies Inc. Built the network communication layer, the automatic update system and the slot-machine game.

Created an XML to Java classes mapping system for Skynamics AG, which was used in the Intershop/Enfinity E-commerce servers.

## 1998

Created a state-machine based user interface and coordination system for Leicher GmbH in Java.

Created a user interface for a cash depositing machine in Delphi for Leicher GmbH.

Created a WYSIWIG text editor component with incremental layout including rotated and round texts for [Envel GmbH](http://www.envel.com/) in C++. The component was used in the label printing program DesignPro 2000 by Zweckform.

Created an automated extraction and reference documentation system for C++ source in Haskell.

## 1996

Contributed to the development of a PC remote control for Ruwido Electronics. Lead the project and built the configuration user interface and parts of the the serial port driver.

## 1995

Co-founded Digital SimpleX GmbH and created the following technologies from 1995 to 1997:

- a user interface kernel for a multimedia authoring system.
- a specific compression method for 256 color pictures, that was optimized for decompression speed and floyd-steinberg dithering.
- a color reduction algorithm, that found a set of best colors based on a number of source picture by using a [Voronoi triangulation](http://en.wikipedia.org/wiki/Voronoi_diagram).
- a high-quality floyd-steinberg implementation that was used to render pictures that were reduced to 256 colors.
- a hyper-text system, including a custom markup, parser and viewer for multimedia applications.

Created a 3D engine including texture mapping and world management by using [Octrees](http://de.wikipedia.org/wiki/Octree). It was used in a DTM realtime game and simulation for Weltbild Verlag.

Created [ACMstreams](http://www.vclcomponents.com/Delphi/Multimedia/dsxacm_zip-info.html), a free audio-compression and format conversion library in Delphi, which was used at Ejay AG, amongst others.

## 1993

Created an animation system that was used in a virtual chemistry lab for Kosmos Verlag.

Contributed to a number of multimedia CD productions that were released by boeder AG between 1993 and 1995.

## 1992

Created [RAP!TOP!COP!](http://www.amigafuture.de/asd.php?asd_id=658&sid=7f91878d785670282df17beb4b45f54b) for Markt & Technik, a real-time hard drive compression driver, a hard drive optimizer, and a disk compression tool, including the appropriate user interfaces for the Amiga, which included the creation of

- two compression algorithms
- an algorithmic method that supports fast random access reads and writes to compressed files
- a compressing Amiga hard-disk driver
- an object-oriented, font-size sensitive user interface engine with automatic layout

## 1989

Created [Oktalyzer](http://www.robotplanet.dk/amiga/oktalyzer/), a music tracker for the Amiga, that was able to play back 8 channels on the 4 Amiga hardware channels.
